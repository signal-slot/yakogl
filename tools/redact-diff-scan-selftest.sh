#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# redact-diff-scan-selftest.sh - tools/redact-diff-scan.py on a throwaway git
# repository in $TMPDIR (plan #388, criterion c7).
#
#   tools/redact-diff-scan-selftest.sh
#
# Positives (each class once, with the path:line and the class checked, and
# the match never printed in full), negatives (documentation addresses,
# loopback, version strings, placeholders), lines only removed or only
# present in the base (not flagged), the `redact-scan: ok` marker, a
# renamed file, a branch behind main, a single-revision range, the default origin/main..HEAD range
# and a range git cannot read (exit 2).
#
# The flagged values are made up and are put together at run time, so this
# file holds none of them as a literal: the private and tailnet addresses
# are an RFC 5737 address with its first octet changed, the names are
# invented. Never use a real address, host name or path here.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
S="$here/redact-diff-scan.py"
T=$(mktemp -d "${TMPDIR:-$here}/redact-diff-scan-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

# Made-up values, assembled so that no literal of them is in this file.
o10=10; o100=100
PRIV4="${o10}.0.2.7"               # a private address (RFC 5737's 192.0.2.7, first octet changed)
TAIL4="$o100.$((64 + 1)).2.7"      # a tailnet address
TS=ts; NET=net
TSNAME="made-up-box.tail-fake.$TS.$NET"
TTYU="/dev/tty""USB0"
TTYA="/dev/tty""ACM3"
TTYS="/dev/tty""S1"
BYID="/dev/serial/by""-id/usb-Fake_Bridge_X-if00-port0"
MAC="02:00:5e"":aa:bb:01"
PW="pass""word"
TK="TOK""EN"
KEYHDR="-----BEGIN OPENSSH PRIV""ATE KEY-----"
MARK="redact-scan"": ok"

g() { git -C "$T/repo" -c user.name=t -c user.email=t@example.invalid "$@"; }
mkdir "$T/repo"
g init -q -b main
# The base already holds a hit: it is neither added nor removed later (a
# context line), and one more hit that the branch removes.
printf 'old line\nkept %s\ndropped %s\n' "$PRIV4" "$PRIV4" > "$T/repo/base.txt"
printf 'a\nb\nc\n' > "$T/repo/move-me.txt"
g add -A; g commit -q -m base
g update-ref refs/remotes/origin/main HEAD
base=$(g rev-parse HEAD)

# A commit with nothing to flag.
cat > "$T/repo/clean.txt" <<'EOF'
docs addresses 192.0.2.1 198.51.100.5 203.0.113.9 and loopback 127.0.0.1
bind 0.0.0.0 port 80; version 1.4.6.2.555 and section 13.10.3.1.2
not an address 999.1.1.1 nor 1.2.3.4.5; spec section 4.6.2.14
if password == other: pass
token=$TOKEN token: <the token>
mac 00:00:00:00:00:00 ff:ff:ff:ff:ff:ff
tty /dev/ttyUSB<N> and /dev/null
let token = next_token();
EOF
# Placeholder values are not credentials (built here so that no other
# scanner of this file takes them for one).
printf '%s="$%s" %s=$GH_%s\n' "$PW" "{BOARD_PW}" "$TK" "$TK" >> "$T/repo/clean.txt"
g add -A; g commit -q -m clean
clean=$(g rev-parse HEAD)

# A commit with one hit per line.
{
  echo "line one is clean"                                   # 1
  echo "host $PRIV4 port 22"                                 # 2 ipv4
  echo "peer $TAIL4"                                         # 3 tailnet-ip
  echo "ssh root@$TSNAME"                                    # 4 tailnet-dns
  echo "uart $TTYU"                                          # 5 tty
  echo "uart $TTYA"                                          # 6 tty
  echo "uart $TTYS"                                          # 7 tty
  echo "uart $BYID"                                          # 8 tty
  echo "ether $MAC"                                          # 9 mac
  echo "BOARD_$PW=madeUpSecret1"                             # 10 secret
  echo "$TK = \"made-up-token-value\""                       # 11 secret
  echo "{\"$PW\": \"made-up-pw\"}"                           # 12 secret
  echo "$KEYHDR"                                             # 13 private-key
  echo "fixture $PRIV4 # $MARK"                              # 14 allowed
  echo "++ $PRIV4 starts with two plus signs"                # 15 ipv4
} > "$T/repo/hits.txt"
# An edit of base.txt: the removed line is not flagged, an added one is.
printf 'old line\nkept %s\nnew %s\n' "$PRIV4" "$TAIL4" > "$T/repo/base.txt"
# A rename with one added line at its end.
g mv move-me.txt moved.txt
printf 'a\nb\nc\nnow %s\n' "$TTYU" > "$T/repo/moved.txt"
g add -A; g commit -q -m hits
hits=$(g rev-parse HEAD)

run() { (cd "$T/repo" && python3 "$S" "$@"); }

# 1. Only the clean commit: exit 0, no output.
set +e; out=$(run "$base..$clean" 2>&1); rc=$?; set -e
[ "$rc" = 0 ] && [ -z "$out" ] && ok "negatives: no hit" || { bad "negatives: rc=$rc"; echo "$out"; }

# 2. The hits commit: exit 1 and exactly these lines.
want="base.txt:3: tailnet-ip: 1***
hits.txt:2: ipv4: 1***
hits.txt:3: tailnet-ip: 1***
hits.txt:4: tailnet-dns: m***
hits.txt:5: tty: /***
hits.txt:6: tty: /***
hits.txt:7: tty: /***
hits.txt:8: tty: /***
hits.txt:9: mac: 0***
hits.txt:10: credential: m***
hits.txt:11: credential: m***
hits.txt:12: credential: m***
hits.txt:13: private-key: -***
hits.txt:15: ipv4: 1***
moved.txt:4: tty: /***"
set +e; out=$(run "$clean..$hits" 2>/dev/null); rc=$?; set -e
[ "$rc" = 1 ] && ok "positives: exit 1" || bad "positives: exit $rc, want 1"
if [ "$out" = "$want" ]; then ok "positives: path:line: class for each hit"
else bad "positives: output differs"; diff <(echo "$want") <(echo "$out") || true; fi

# 3. Nothing is printed in full.
leak=0
for v in "$PRIV4" "$TAIL4" "$TSNAME" "$TTYU" "$BYID" "$MAC" madeUpSecret1 made-up-token-value made-up-pw; do
  case "$out" in *"$v"*) leak=1 ;; esac
done
[ "$leak" = 0 ] && ok "matches are masked" || bad "a match is printed in full"

# 4. A single revision is REV..HEAD; the default range is origin/main..HEAD.
set +e; out1=$(run "$clean" 2>/dev/null); rc1=$?
out2=$(run 2>/dev/null); rc2=$?; set -e
[ "$rc1" = 1 ] && [ "$out1" = "$want" ] && ok "REV means REV..HEAD" || bad "REV: rc=$rc1"
[ "$rc2" = 1 ] && [ "$out2" = "$want" ] && ok "default range origin/main..HEAD" || bad "default: rc=$rc2"

# 5. With origin/main at the tip, nothing is added: exit 0.
g update-ref refs/remotes/origin/main "$hits"
set +e; out=$(run 2>&1); rc=$?; set -e
[ "$rc" = 0 ] && [ -z "$out" ] && ok "empty range: exit 0" || bad "empty range: rc=$rc"

# 6. A range git cannot read: exit 2.
set +e; run "no-such-rev..HEAD" >/dev/null 2>&1; rc=$?; set -e
[ "$rc" = 2 ] && ok "bad range: exit 2" || bad "bad range: exit $rc, want 2"

# 7. A branch behind main is not charged with what main added since (nor
# with main's removed lines): A..B is read as A...B.
g branch behind "$clean"
set +e; out=$(run "origin/main..behind" 2>&1); rc=$?; set -e
[ "$rc" = 0 ] && [ -z "$out" ] && ok "branch behind main: exit 0" || { bad "behind: rc=$rc"; echo "$out"; }

if [ "$fail" = 0 ]; then echo "redact-diff-scan selftest: all ok"; else echo "redact-diff-scan selftest: FAILED"; fi
exit "$fail"
