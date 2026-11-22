#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of tools/redact-diff-scan.py on a throwaway repository.
# Fixture values are RFC 5737 / made-up and built from pieces, so this file
# holds no pattern itself.
set -u
here=$(cd "$(dirname "$0")" && pwd)
scan="$here/redact-diff-scan.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/redact-scan.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
cd "$tmp" && git init -q . || exit 2
git config user.email t@example.invalid
git config user.name t
echo base > a.txt
git add a.txt
git commit -qm base
base=$(git rev-parse HEAD)
fail=0

# run <name> <expected rc> <line>: commit one added line, scan it.
run() {
  printf '%s\n' "$3" > f.txt
  git add f.txt
  git commit -qm "$1"
  python3 -I "$scan" "$base..HEAD" > out.txt 2>&1
  rc=$?
  if [ "$rc" != "$2" ]; then echo "FAIL $1: rc=$rc want $2"; cat out.txt; fail=1; else echo "ok   $1"; fi
  git reset -q --hard "$base"
}
dot=.
ip() { echo "$1$dot$2$dot$3$dot$4"; }
tailnet="$dot""ts""$dot""net"
run ipv4-private 1 "host $(ip 10 1 2 3)"
run ipv4-192 1 "host $(ip 192 168 5 9)"
run ipv4-public 1 "to $(ip 8 8 8 8)"
run tailnet-ip 1 "peer $(ip 100 100 1 2)"
run tailnet-name 1 "ssh box1-fake$tailnet"
run uart-usb 1 "console /dev/tty""USB0"
run uart-acm 1 "console /dev/tty""ACM3"
run uart-by-id 1 "/dev/serial/by-""id/usb-FAKE_0001-if00"
run mac 1 "eth 02:00:5e:10:00:01"  # redact-scan: ok
run password 1 "password=hunter22"  # redact-scan: ok
run token 1 "TOKEN: abcd1234efgh"  # redact-scan: ok
run privkey 1 "-----BEGIN OPENSSH PRIVATE"" KEY-----"
run doc-1 0 "host $(ip 192 0 2 7)"
run doc-2 0 "host $(ip 198 51 100 7)"
run doc-3 0 "host $(ip 203 0 113 7)"
run any-addr 0 "bind $(ip 0 0 0 0)"
run loopback 0 "bind $(ip 127 0 0 1)"
run version 0 "release 1.2.3 and 2026-11-22"
run compare 0 "if password == other"
run marker 0 "x $(ip 10 1 2 3) # redact-scan:"" ok"

# a removed line is not scanned
printf '%s\n' "old $(ip 10 1 2 3)" > g.txt
git add g.txt
git commit -qm old
base=$(git rev-parse HEAD)
git rm -q g.txt
git commit -qm rm
if python3 -I "$scan" "$base..HEAD" > /dev/null 2>&1; then echo "ok   removed"; else echo "FAIL removed"; fail=1; fi

# file:line output, and a bad range exits 2
printf 'a\nb %s\n' "$(ip 10 1 2 3)" > h.txt
git add h.txt
git commit -qm h
if python3 -I "$scan" "$base..HEAD" 2> /dev/null | grep -q '^h.txt:2: ipv4:'; then echo "ok   location"; else echo "FAIL location"; fail=1; fi
python3 -I "$scan" nonexistent..range > /dev/null 2>&1
if [ $? = 2 ]; then echo "ok   badrange"; else echo "FAIL badrange"; fail=1; fi

[ $fail = 0 ] && echo "redact-diff-scan selftest: ok"
exit $fail
