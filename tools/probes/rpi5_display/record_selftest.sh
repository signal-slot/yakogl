#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host self-test of record.sh. No board, no network.
#   bash tools/probes/rpi5_display/record_selftest.sh

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REC=$HERE/record.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/record_selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
FAIL=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; FAIL=$((FAIL + 1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

# values made up here: no literal address or device path sits in this tree
IP="$((RANDOM % 80 + 20)).$((RANDOM % 200 + 20)).7.$((RANDOM % 200 + 20))"
IP6="$(printf "fe%s" 80):0:0:0:$RANDOM:$RANDOM:$RANDOM:$RANDOM"
HOST="hq$RANDOM-node"
USR="u$RANDOM"
UART="/dev/$(printf 't%s' ty)FAKE$RANDOM"
HOMED="/$(printf 'ho%s' me)/$USR/work"

D=$(RECORDS_DIR="$T/records" bash "$REC" new rpi5_display | tail -n 1)
check "new: directory named <stamp>-<name>-<sha8>" '[ -d "$D" ] && [[ $(basename "$D") =~ ^[0-9]{8}-[0-9]{4}-rpi5_display-[0-9a-f]{8}$ ]]'
check "new: bad name refused" '! RECORDS_DIR="$T/records" bash "$REC" new "a/b" >/dev/null 2>&1'

echo "throttled=0x50005" | bash "$REC" throttled "$D" before
echo "garbage" | bash "$REC" throttled "$D" after
check "throttled: value and unknown recorded" '[ "$(cat "$D/throttled.txt")" = "$(printf "before 0x50005\nafter unknown")" ]'

printf 'host %s user %s\nip %s ip6 %s\nuart %s\ncwd %s\nplain text\n' "$HOST" "$USR" "$IP" "$IP6" "$UART" "$HOMED" >"$D/a.txt"
mkdir "$D/sub"
printf 'again %s\n' "$IP" >"$D/sub/b.txt"
check "check refuses a dirty record" '! bash "$REC" check "$D" 2>/dev/null'
REDACT_NAMES="$HOST,$USR" bash "$REC" redact "$D"
check "redact: check passes" 'bash "$REC" check "$D"'
leaks=0
for v in "$IP" "$IP6" "$HOST" "$USR" "$UART" "$HOMED"; do
	grep -rlF -- "$v" "$D" >/dev/null && leaks=1
done
check "redact: no value left in any file, subdirectories included" '[ $leaks -eq 0 ]'
check "redact: other text kept" 'grep -q "plain text" "$D/a.txt" && grep -q "before 0x50005" "$D/throttled.txt"'

echo
if [ $FAIL -eq 0 ]; then echo "record_selftest: all ok"; exit 0; fi
echo "record_selftest: $FAIL failed"
exit 1
