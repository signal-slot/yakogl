#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of tools/redact-diff-scan.py on a throwaway repository.
#
# Every fixture value a class matches is built from pieces: a dotted address
# from an ip() call, a keyword or name from a variable, a long literal cut
# in two. So neither this file nor the tool holds a matchable value -- and
# both live where scans read them: the last check commits copies of the two
# files and scans them clean. Fixture values are RFC 5737 addresses and
# made-up names.
set -u
here=$(cd "$(dirname "$0")" && pwd)
scan="$here/redact-diff-scan.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/redact-diff-scan.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

cd "$tmp" && git init -q . || exit 2
git config user.email t@example.invalid
git config user.name t
echo base > a.txt
git add a.txt
git commit -qm base
base=$(git rev-parse HEAD)

# commit_line <text...>: commit one file whose single line is the text.
commit_line() {
  printf '%s\n' "$*" > f.txt
  git add f.txt
  git commit -qm fixture
}

# run <name> <expected rc> <line>: commit the line, scan base..HEAD, compare.
run() {
  commit_line "$3"
  python3 -I "$scan" "$base..HEAD" > out.txt 2>&1
  rc=$?
  if [ "$rc" = "$2" ]; then
    ok "$1"
  else
    bad "$1: rc=$rc want $2"
    cat out.txt
  fi
  git reset -q --hard "$base"
}

dot=.
ip() { echo "$1$dot$2$dot$3$dot$4"; }
tailnet="$dot""ts""$dot""net"            # the DNS suffix, in pieces
som="verdin-am62p-1a2b3c4""d"           # the last hex digit cut off in source
som8mp="verdin-imx8mp-a1b2c3d""4"
snk="Serial""Number"                    # the kernel's USB serial name, in two
iser="i""Serial"
pw="pass""word"
tok="TO""KEN"
akw="api""_key"

# --- addresses --------------------------------------------------------------
run ipv4-private 1 "host $(ip 10 1 2 3)"
run ipv4-192 1 "host $(ip 192 168 5 9)"
run ipv4-172 1 "lab $(ip 172 20 1 1)"
run ipv4-link 1 "link-local $(ip 169 254 1 1)"
run ipv4-public 1 "to $(ip 8 8 8 8)"
run doc-1 0 "host $(ip 192 0 2 7)"
run doc-2 0 "host $(ip 198 51 100 7)"
run doc-3 0 "host $(ip 203 0 113 7)"
run any-addr 0 "bind $(ip 0 0 0 0)"
run loopback 0 "bind $(ip 127 0 0 1)"
run version 0 "release 1.2.3 and 2026-11-23"
run version-glued 0 "yakogl v26.1.2.3.4 is not an address"
run deqp-version 0 "dEQP Core vulkan-cts-$(ip 1 4 6 2)-555-g0123456789ab"
run deqp-real 1 "dEQP Core vulkan-cts-$(ip 1 4 6 2)-555-g0123456789ab on $(ip 10 9 8 7)"

# the tailnet range reads as its own class, not as the plain one
commit_line "peer $(ip 100 100 1 2)"
python3 -I "$scan" "$base..HEAD" > tl.txt 2>&1
if grep -q ': tailnet-ipv4:' tl.txt && ! grep -q ': ipv4:' tl.txt; then
  ok "tailnet-class"
else
  bad "tailnet-class"; cat tl.txt
fi
git reset -q --hard "$base"

# --- names ------------------------------------------------------------------
run tailnet-name 1 "ssh box1-fake$tailnet"
run tailnet-safe 0 "the $tailnet suffix is the class"
run som-name 1 "board $som"
run som-name-8mp 1 "board $som8mp"
run som-zero 0 "board verdin-am62p-000000""0"
run som-zero-8mp 0 "board verdin-imx8mp-000000""0"
run som-long 0 "board verdin-am62p-1a2b3c4d5 has nine digits"
rhost=fake-host.example
run root-at 1 "ssh root@$rhost"
run root-ip 1 "ping root@$(ip 192 0 2 7)"
run root-safe 0 "ssh root@<host> to \$target"

# --- device paths, serials, ports -------------------------------------------
run uart-usb 1 "console /dev/tty""USB0"
run uart-acm 1 "console /dev/tty""ACM3"
run uart-s 1 "board console /dev/tty""S2"
run uart-by-id 1 "/dev/serial/by-""id/usb-FAKE_0001-if00"
run uart-safe 0 "console /dev/tty0 and /dev/console"
run usb-serial 1 "usb 1-1: $snk: A12345"
run usb-iserial 1 "udev $iser=424242"
run usb-novalue 0 "lsusb prints no $snk line here"
run bridge 1 "console over ssh -p 3593""9"
run bridge-safe 0 "the bridge ports are the 3593x run"

# --- credentials ------------------------------------------------------------
run mac 1 "eth 02:00:5e:10:00:0""1"
run mac-safe 0 "time 00:11:22:33:44"
run credential 1 "$pw=hunter22"
run credential-space 1 "$pw = hunter23"
run credential-eq 0 "if $pw == other then true"
run credential-token 1 "$tok: abcd1234efgh"
run credential-apikey 1 "$akw = hunter-key-value"
run api-token 1 "gh""p_$(printf 'A%.0s' {1..36})"
run privkey 1 "-----BEGIN OPENSSH PRIVATE"" KEY-----"

# a line holding two classes prints both
commit_line "console /dev/tty""USB0 at $(ip 10 1 2 3)"
python3 -I "$scan" "$base..HEAD" > two.txt 2>&1
if grep -q ': uart-path:' two.txt && grep -q ': ipv4:' two.txt; then
  ok "two-classes"
else
  bad "two-classes"; cat two.txt
fi
git reset -q --hard "$base"

# a marker line is skipped, a removed line is not scanned
run marker 0 "x $(ip 10 1 2 3) # redact-scan:"" ok"
commit_line "old $(ip 10 1 2 3)"
git rm -q f.txt
git commit -qm rm
if python3 -I "$scan" "$(git rev-parse HEAD~1)..HEAD" > /dev/null 2>&1; then
  ok "removed"
else
  bad "removed: a deleted line was scanned"
fi
git reset -q --hard "$base"

# file:line points at the added line
printf 'a\nb %s\n' "$(ip 10 1 2 3)" > h.txt
git add h.txt
git commit -qm h
python3 -I "$scan" "$base..HEAD" > loc.txt 2>&1
if grep -q '^h.txt:2: ipv4:' loc.txt; then ok "location"; else bad "location"; cat loc.txt; fi
git reset -q --hard "$base"

# a bad range exits 2 and names it; so do extra arguments; --help exits 0
python3 -I "$scan" nonexistent..range > /dev/null 2> err.txt
if [ $? = 2 ] && grep -q nonexistent err.txt; then ok "badrange"; else bad "badrange"; cat err.txt; fi
python3 -I "$scan" a b > /dev/null 2>&1
if [ $? = 2 ]; then ok "extraargs"; else bad "extraargs"; fi
if python3 -I "$scan" --help 2>&1 | grep -qi 'Scan the added lines'; then
  ok "help"
else
  bad "help"
fi

# the default range is origin/main..HEAD: a benign commit sets the remote
# ref, the next commit must be found through it, and the range reads empty
# again once HEAD moves back to the ref.
commit_line "note $(ip 192 0 2 9) is documentation"
b=$(git rev-parse HEAD)
git update-ref refs/remotes/origin/main "$b"
commit_line "peer $(ip 10 1 2 3)"
if python3 -I "$scan" > /dev/null 2>&1; then
  bad "default: origin/main..HEAD missed the added private address"
else
  git reset -q --hard "$b"
  if python3 -I "$scan" > /dev/null 2>&1; then ok "default"; else bad "default: empty range"; fi
fi
git reset -q --hard "$base"

# the tool's own source and this self-test, committed to a throwaway tree,
# scan clean: the files live where a scan reads them, so neither may hold a
# value its classes match.
cp "$scan" scan-copy.py
cp "$here/redact-diff-scan-selftest.sh" self-copy.sh
git add scan-copy.py self-copy.sh
git commit -qm copies
python3 -I "$scan" "$base..HEAD" > own.txt 2>&1
if [ $? = 0 ] && [ ! -s own.txt ]; then
  ok "own"
else
  bad "own: the scanner or this self-test holds a matchable value"
  cat own.txt
fi
git reset -q --hard "$base"

if [ "$fail" = 0 ]; then echo "redact-diff-scan selftest: ok"; else echo "redact-diff-scan selftest: FAIL"; fi
exit "$fail"
