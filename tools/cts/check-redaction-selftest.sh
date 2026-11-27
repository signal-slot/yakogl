#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# check-redaction-selftest.sh - tools/cts/check-redaction.py on a throwaway
# git repository in $TMPDIR (plan #388, criterion c7).
#
# Checks the classes the script adds to tools/redact-diff-scan.py (ipv6,
# usb-serial, ssh-host) with file:line, the inherited ones once (ipv4,
# tailnet), placeholders that must pass ($VAR, example.invalid, 192.0.2.x,
# 2001:db8::), the file-list mode and exit codes.
#
# Flagged values are made up and assembled at run time: this file holds none
# of them as a literal. Never put a real address or name here.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
S="$here/check-redaction.py"
T=$(mktemp -d "${TMPDIR:-$here}/check-redaction-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

o=10
PRIV4="$o.0.2.7"
V6="fd00:"":1234:5:6:7"
LL6="fe80""::7"
SER="ID_SERIAL_SHORT""=A1B2C3D4E5"
ISER="iSerial 3 ""FAKE0123456"
ULINK="usb-Fake_Bridge_Y_""ABCD1234-if00"
SSHBAD="ssh root""@host-1.lab.invalid2"
SCPBAD="scp a.bin pi""@made-up-box:/x"

repo="$T/r"
git init -q "$repo"
cd "$repo"
git config user.email t@example.invalid
git config user.name t
echo base > f.txt
git add f.txt
git commit -qm base
git branch -q -M main
git update-ref refs/remotes/origin/main HEAD
cat > g.txt <<EOT
clean line
addr $PRIV4 here
v6 $V6
ll $LL6
$SER
$ISER
link $ULINK
$SSHBAD
$SCPBAD
EOT
git add g.txt
git commit -qm add

out=$("$S" 2>&1) && rc=0 || rc=$?
[ "$rc" = 1 ] && ok "default range: exit 1" || bad "default range exit $rc"
chk() { # line class
    grep -q "^g.txt:$1: $2: " <<<"$out" && ok "g.txt:$1 $2" || bad "g.txt:$1 $2 missing"
}
chk 2 ipv4; chk 3 ipv6; chk 4 ipv6; chk 5 usb-serial; chk 6 usb-serial
chk 7 usb-serial; chk 8 ssh-host; chk 9 ssh-host
grep -q '^g.txt:1:' <<<"$out" && bad "clean line flagged" || ok "clean line passes"
grep -q "$PRIV4" <<<"$out" && bad "match printed in full" || ok "match masked"

cat > h.txt <<'EOT'
ssh root@$BOARD_HOST
ssh torizon@<board>
ssh root@example.invalid
ssh git@github.com
ssh user@192.0.2.9
addr 192.0.2.5 and 2001:db8::1 and ::1 and std::vector::new
ID_SERIAL_SHORT=$SERIAL
password=$PW
EOT
"$S" --files h.txt && ok "placeholders pass (files mode)" || bad "placeholder flagged"
"$S" --files g.txt >/dev/null 2>&1 && bad "files mode misses" || ok "files mode flags g.txt"
git add h.txt; git commit -qm ph
n=$("$S" HEAD~1...HEAD 2>&1 | grep -c . || true)
[ "$n" = 0 ] && ok "placeholder commit range clean" || bad "range flagged $n lines"
"$S" nosuchrev >/dev/null 2>&1 && rc=0 || rc=$?
[ "$rc" = 2 ] && ok "bad range: exit 2" || bad "bad range exit $rc"
[ "$fail" = 0 ] && echo "check-redaction selftest: all ok" || { echo "check-redaction selftest: FAILED"; exit 1; }
