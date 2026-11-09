#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-board-state.sh without a board: a fake ssh on
# PATH (testdata/mali-board-state/bin/ssh) serves canned outputs from
# testdata/mali-board-state/<case>/ over the "good" case. It covers unchanged,
# a /boot file changed, a module added, fw_printenv unavailable (in both and in
# one snapshot), ssh unreachable (exit 4), that every command sent is a reading
# one, and that the script text has no write, sudo or module verb.
#
#   tools/cts/mali-board-state-selftest.sh   prints "mali-board-state-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-board-state
SCRIPT=$HERE/mali-board-state.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-board-state-selftest-XXXXXX")
trap 'rm -rf "$tmp"' EXIT
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
mkdir -p "$tmp/bin"
cp "$FIX/bin/ssh" "$tmp/bin/ssh"
chmod +x "$tmp/bin/ssh"

# snap <name> <case>: snapshot of a case into $tmp/<name>.snap; rc in $tmp/<name>.rc
snap() {
  local name=$1 case=$2
  rm -rf "$tmp/case-$name"
  mkdir -p "$tmp/case-$name"
  cp "$FIX"/good/* "$tmp/case-$name/"
  [ "$case" = good ] || cp "$FIX/$case"/* "$tmp/case-$name/"
  : > "$tmp/$name.log"
  env PATH="$tmp/bin:$PATH" FAKE_CASE="$tmp/case-$name" FAKE_LOG="$tmp/$name.log" \
    bash "$SCRIPT" snapshot root@board.invalid "$tmp/$name.snap" > "$tmp/$name.out" 2>&1
  echo $? > "$tmp/$name.rc"
}
cmp_run() { # <name> <before> <after>
  bash "$SCRIPT" compare "$tmp/$2.snap" "$tmp/$3.snap" > "$tmp/$1.out" 2>&1
  echo $? > "$tmp/$1.rc"
}
rc_is() { [ "$(cat "$tmp/$1.rc")" = "$2" ]; }
has() { grep -qF -- "$2" "$tmp/$1.out"; }
lacks() { ! grep -qF -- "$2" "$tmp/$1.out"; }

snap a good
snap b good
snap boot boot-changed
snap mod module-added
snap nofw no-fw-printenv
snap down ssh-unreachable

check "snapshot exits 0" rc_is a 0
for k in boot_files boot_sha256 uboot_env modules modules_sha256 boot_id kernel; do
  check "snapshot has $k" grep -q "^$k=." "$tmp/a.snap"
done
check "snapshot counts 3 /boot files" grep -qx 'boot_files=3' "$tmp/a.snap"
check "snapshot counts 3 modules" grep -qx 'modules=3' "$tmp/a.snap"
check "snapshot has the kernel" grep -qx 'kernel=6.18.55-current-rockchip64' "$tmp/a.snap"
check "env hash is a sha256" grep -qE '^uboot_env=[0-9a-f]{64}$' "$tmp/a.snap"
check "no-fw-printenv snapshot says unavailable" grep -qx 'uboot_env=unavailable' "$tmp/nofw.snap"

cmp_run same a b
check "unchanged: exit 0" rc_is same 0
check "unchanged: /boot line" has same "mali-board-state: /boot unchanged (3 files, sha256 "
check "unchanged: modules line" has same "mali-board-state: modules unchanged (3 modules"
check "unchanged: env line" has same "mali-board-state: U-Boot env unchanged"
check "unchanged: no CHANGED" lacks same CHANGED

cmp_run boot a boot
check "boot file changed: exit 1" rc_is boot 1
check "boot file changed: /boot CHANGED" has boot "mali-board-state: /boot CHANGED"
check "boot file changed: modules unchanged" has boot "modules unchanged"

cmp_run mod a mod
check "module added: exit 1" rc_is mod 1
check "module added: modules CHANGED" has mod "mali-board-state: modules CHANGED (before 3 modules"
check "module added: /boot unchanged" has mod "/boot unchanged"

cmp_run nofw nofw nofw
check "fw_printenv unavailable in both: exit 0" rc_is nofw 0
check "fw_printenv unavailable in both: said so" has nofw "U-Boot env unavailable in both"
cmp_run nofw1 a nofw
check "env available then not: exit 1" rc_is nofw1 1
check "env available then not: CHANGED" has nofw1 "U-Boot env CHANGED"

check "ssh unreachable: exit 4" rc_is down 4
check "ssh unreachable: clear message" has down "board not reachable over ssh"
check "ssh unreachable: no snapshot file" test ! -e "$tmp/down.snap"
bash "$SCRIPT" compare "$tmp/none.snap" "$tmp/a.snap" > "$tmp/nofile.out" 2>&1
echo $? > "$tmp/nofile.rc"
check "compare of a missing file: exit 2" rc_is nofile 2

# reading_only <log>: every command sent is one of the known reading commands
reading_only() {
  local line
  while IFS= read -r line; do
    case $line in
      'echo yakogl-state-ok' | 'cat /proc/modules' | 'cat /proc/sys/kernel/random/boot_id' | 'uname -r') ;;
      'find /boot -xdev -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum') ;;
      'if command -v fw_printenv > /dev/null 2>&1; then fw_printenv 2> /dev/null || echo yakogl-state-unavailable; else echo yakogl-state-unavailable; fi') ;;
      *) echo "not allowed: $line" >&2; return 1 ;;
    esac
  done < "$1"
  [ -s "$1" ]
}
check "commands sent are reading ones" reading_only "$tmp/a.log"
printf 'echo x > /boot/y\n' > "$tmp/bad.log"
rejects_write() { ! reading_only "$1"; }
check "reading check rejects a write" rejects_write "$tmp/bad.log"
no_verbs() { ! grep -v '^ *#' "$1" | grep -qE '(^|[^[:alnum:]_-])(sudo|modprobe|rmmod|insmod|tee|rm|mv|cp|dd|mount|umount|fw_setenv)([^[:alnum:]_-]|$)'; }
check "script text has no write/sudo/module verb" no_verbs "$SCRIPT"

echo "mali-board-state-selftest: $passed checks passed"
[ "$failed" = 0 ]
