#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# check-redaction-selftest.sh - tools/cts/check-redaction.py on a throwaway
# git repository in $TMPDIR (plan #388, criterion c7).
#
#   tools/cts/check-redaction-selftest.sh
#
# The classes the script adds to tools/redact-diff-scan.py (ipv6, usb-serial,
# ssh-host, each with its path:line and class checked), one inherited class
# (ipv4), the masking, placeholders that must pass ($VAR, ${VAR}, <name>,
# example.invalid, the documentation ranges, ::1, 2001:db8::1, github.com),
# the file-list mode, the default origin/main...HEAD range, a commit that
# only adds placeholders (exit 0) and a range git cannot read (exit 2).
#
# The flagged values are made up and assembled at run time, so this file
# holds none of them as a literal (tools/release/redaction-scan.py scans the
# whole tree, this file included): the ssh forms that must be flagged are
# put together by concatenation, and the two placeholder lines that any
# credential or login scan would read as one are appended to the fixture at
# run time. Never put a real address, host name, serial or password here.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
S="$here/check-redaction.py"
T=$(mktemp -d "${TMPDIR:-$here}/check-redaction-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

# Made-up values, assembled so that no literal of them is in this file.
o10=10
PRIV4="${o10}.0.2.7"                        # a private address (RFC 5737's 192.0.2.7, first octet changed)
V6="fd00:"":1234:5:6:7"                     # a ULA address
LL6="fe80""::7"                             # a link-local address
SER="ID_SERIAL_SHORT""=A1B2C3D4E5"          # a udev serial property
ISER="iSerial 3 ""FAKE0123456"              # an lsusb serial string
ULINK="usb-Fake_Bridge_Y_""ABCD1234-if00"   # a usb-serial by-id link tail
SSHBAD="ssh root""@host-1.lab.invalid2"
SCPBAD="scp a.bin pi""@made-up-box:/x"
USR="us""er"; PWK="pass""word"

g() { git -C "$T/repo" -c user.name=t -c user.email=t@example.invalid "$@"; }
mkdir "$T/repo"
g init -q -b main
echo base > "$T/repo/f.txt"
g add -A; g commit -q -m base
g update-ref refs/remotes/origin/main HEAD

{
  echo "clean line"                          #  1
  echo "addr $PRIV4 here"                    #  2 ipv4
  echo "v6 $V6"                              #  3 ipv6
  echo "ll $LL6"                             #  4 ipv6
  echo "$SER"                                #  5 usb-serial
  echo "$ISER"                               #  6 usb-serial
  echo "link $ULINK"                         #  7 usb-serial
  echo "$SSHBAD"                             #  8 ssh-host
  echo "$SCPBAD"                             #  9 ssh-host
} > "$T/repo/g.txt"
g add -A; g commit -q -m hits

run() { (cd "$T/repo" && python3 "$S" "$@"); }

# 1. The default range over the hits commit: exit 1, one line per class.
set +e; out=$(run 2>&1); rc=$?; set -e
[ "$rc" = 1 ] && ok "default range: exit 1" || bad "default range: rc=$rc, want 1"
chk() { # <line> <class>
  grep -q "^g.txt:$1: $2: " <<<"$out" && ok "g.txt:$1 $2" \
    || { bad "g.txt:$1 $2 missing"; echo "$out"; }
}
chk 2 ipv4; chk 3 ipv6; chk 4 ipv6
chk 5 usb-serial; chk 6 usb-serial; chk 7 usb-serial
chk 8 ssh-host; chk 9 ssh-host
grep -q "^g.txt:1:" <<<"$out" && bad "clean line flagged" || ok "clean line passes"

# 2. Nothing is printed in full.
leak=0
for v in "$PRIV4" "$V6" "$LL6" A1B2C3D4E5 FAKE0123456 "$ULINK" host-1.lab.invalid2 made-up-box; do
  case "$out" in *"$v"*) leak=1 ;; esac
done
[ "$leak" = 0 ] && ok "matches are masked" || bad "a match is printed in full"

# 3. Placeholders: every line of the fixture must pass.
cat > "$T/repo/h.txt" <<'EOT'
ssh root@$BOARD_HOST
ssh torizon@<board>
ssh root@example.invalid
ssh git@github.com
addr 192.0.2.5 and 2001:db8::1 and ::1 and std::vector::new
token=$TOKEN
ID_SERIAL_SHORT=$SERIAL
EOT
# The three placeholder lines this file must not hold as literals (a
# credential key with a value of its own, twice, and a login at a dotted
# address): the two PWK lines append a shell-variable value and an empty
# brace value, and the USR line an ssh to a documentation-range address.
printf '%s=$PW\n' "$PWK" >> "$T/repo/h.txt"
printf '%s=${BOARD_PW}\n' "$PWK" >> "$T/repo/h.txt"
printf 'ssh %s@192.0.2.9\n' "$USR" >> "$T/repo/h.txt"
set +e; out=$(run --files h.txt 2>&1); rc=$?; set -e
[ "$rc" = 0 ] && [ -z "$out" ] && ok "placeholders pass (--files mode)" \
  || { bad "placeholders: rc=$rc"; echo "$out"; }
set +e; out=$(run --files g.txt 2>/dev/null); rc=$?; set -e
[ "$rc" = 1 ] && ok "files mode flags g.txt" || bad "files mode: rc=$rc, want 1"

# 4. A commit that only adds placeholders: exit 0 over its range.
g add -A; g commit -q -m placeholders
set +e; out=$(run "HEAD~1...HEAD" 2>&1); rc=$?; set -e
[ "$rc" = 0 ] && ok "placeholder commit range clean" || { bad "range: rc=$rc"; echo "$out"; }

# 5. A range git cannot read: exit 2.
set +e; run "no-such-rev" >/dev/null 2>&1; rc=$?; set -e
[ "$rc" = 2 ] && ok "bad range: exit 2" || bad "bad range: rc=$rc, want 2"

if [ "$fail" = 0 ]; then echo "check-redaction selftest: all ok"; else echo "check-redaction selftest: FAILED"; fi
exit "$fail"
