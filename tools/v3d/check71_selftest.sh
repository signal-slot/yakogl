#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins the exit status and summary of tools/v3d/check71.sh on its ok, gap and
# failure paths, with scripts made in temporary directories (CHECK71_DIR):
# all pass -> 0; one exits 77 -> 77 naming it; a failure and a gap -> 1;
# a non-executable file is skipped; the counts in the summary are right.
set -u
cd "$(dirname "$0")/../.." || exit 2
root=$PWD/build/v3d71/selftest
rm -rf "$root"
mkdir -p "$root" || exit 2
fails=0
checks=0
mk() { # dir name exit-status [mode]
  printf '#!/bin/sh\necho "%s said hi"\nexit %s\n' "$2" "$3" >"$1/$2.sh"
  chmod "${4:-755}" "$1/$2.sh"
}
run() { # script-dir: sets out and rc
  out=$(CHECK71_DIR=$1 tools/v3d/check71.sh 2>&1)
  rc=$?
}
expect() { # label rc-want summary-want [line-want]
  checks=$((checks + 1))
  if [ "$rc" != "$2" ]; then
    echo "FAIL $1: exit $rc, expected $2"
    fails=$((fails + 1))
  elif ! printf '%s\n' "$out" | grep -qxF "check71: $3"; then
    echo "FAIL $1: summary not 'check71: $3'"
    fails=$((fails + 1))
  elif [ -n "${4:-}" ] && ! printf '%s\n' "$out" | grep -qxF "$4"; then
    echo "FAIL $1: no line '$4'"
    fails=$((fails + 1))
  fi
}

d=$root/pass
mkdir -p "$d"
mk "$d" a 0
mk "$d" b 0
run "$d"
expect "all pass" 0 "2 ok, 0 failed, 0 gap(s)"

d=$root/gap
mkdir -p "$d"
mk "$d" a 0
mk "$d" b 77
run "$d"
expect "one gap" 77 "1 ok, 0 failed, 1 gap(s)" "gap: b"

d=$root/fail
mkdir -p "$d"
mk "$d" a 1
mk "$d" b 77
mk "$d" c 0
run "$d"
expect "failure and gap" 1 "1 ok, 1 failed, 1 gap(s)" "failed: a"

d=$root/skip
mkdir -p "$d"
mk "$d" a 0
mk "$d" b 1 644
run "$d"
expect "non-executable skipped" 0 "1 ok, 0 failed, 0 gap(s)"

d=$root/empty
mkdir -p "$d"
run "$d"
expect "empty directory" 0 "0 ok, 0 failed, 0 gap(s)"

rm -rf "$root"
if [ "$fails" -ne 0 ]; then
  echo "$((checks - fails)) of $checks passed"
  exit 1
fi
echo "$checks of $checks passed"
