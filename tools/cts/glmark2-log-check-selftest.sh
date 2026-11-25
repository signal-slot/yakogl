#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of glmark2-log-check.sh on synthetic log directories (documentation
# values only) built under $TMPDIR, else mktemp -d in the workspace; never /tmp.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
chk=$HERE/glmark2-log-check.sh
if [ -n "${TMPDIR:-}" ]; then
    work=$(mktemp -d "$TMPDIR/glmark2-log-check.XXXXXX")
else
    work=$(mktemp -d "$HERE/../../.glmark2-log-check.XXXXXX")
fi
trap 'chmod -R u+rwX "$work" 2>/dev/null; rm -rf "$work"' EXIT
bad=0
n=0

mk() { # a clean directory
    mkdir -p "$work/$1"
    for f in tally-table.txt tally-results-row.txt state-compare.txt; do
        echo "scene build ran 12.5 fps" > "$work/$1/$f"
    done
}
# expect <name> <exit> <pattern in output> [<pattern that must be absent>]
expect() {
    local name=$1 want=$2 pat=$3 absent=${4:-} out rc
    n=$((n + 1))
    out=$(bash "$chk" "$work/$name" 2>&1)
    rc=$?
    if [ "$rc" -ne "$want" ] || ! grep -q -- "$pat" <<<"$out" ||
        { [ -n "$absent" ] && grep -q -- "$absent" <<<"$out"; }; then
        echo "FAIL $name: exit $rc (want $want), output:"
        echo "$out"
        bad=$((bad + 1))
    else
        echo "ok   $name"
    fi
}

mk clean
expect clean 0 '^glmark2-log-check: ok$'

mk nostate
rm "$work/nostate/state-compare.txt"
expect nostate 1 'c1 layout: FAIL missing state-compare.txt'

mk ip
echo "connect 192.0.2.1 port" >> "$work/ip/tally-table.txt"
expect ip 1 'tally-table.txt:2' '192\.0\.2'

mk mac
echo "link 02:00:5e:10:00:01" >> "$work/mac/state-compare.txt"
expect mac 1 'state-compare.txt:2' '02:00:5e'

mk home
echo "path /home/alice/x" >> "$work/home/tally-results-row.txt"
expect home 1 'tally-results-row.txt:2' 'alice'

mk tty
echo "uart /dev/ttyUSB0" >> "$work/tty/tally-table.txt"
expect tty 1 'c2 redaction: FAIL' 'ttyUSB'

mk mail
echo "bob@board-1.example" >> "$work/mail/tally-table.txt"
expect mail 1 'c2 redaction: FAIL' 'bob@'

mk sub
mkdir "$work/sub/raw"
echo "10.0.0.7" > "$work/sub/raw/a.log"
expect sub 1 'raw/a.log:1' '10\.0\.0'

# A binary file (NUL bytes) holding an address must fail, not be skipped.
mk bin
printf 'x\0y\n192.0.2.1\n\0' > "$work/bin/dump.bin"
expect bin 1 'c2 redaction: FAIL' '192\.0\.2'

# An unreadable file is a grep error: c2 fails (not testable as root).
mk unread
echo "clean" > "$work/unread/secret.log"
chmod 000 "$work/unread/secret.log"
if [ -r "$work/unread/secret.log" ]; then
    echo "skip unread: file stays readable here (root)"
else
    expect unread 1 'c2 redaction: FAIL grep error'
fi

mk both
rm "$work/both/tally-table.txt"
echo "192.0.2.9" >> "$work/both/state-compare.txt"
expect both 1 'glmark2-log-check: FAIL 2'

n=$((n + 1))
bash "$chk" > /dev/null 2>&1
rc=$?
if [ "$rc" -eq 2 ]; then
    echo "ok   usage"
else
    echo "FAIL usage: exit $rc"
    bad=$((bad + 1))
fi

if [ "$bad" -eq 0 ]; then
    echo "glmark2-log-check-selftest: ok ($n cases)"
    exit 0
fi
echo "glmark2-log-check-selftest: FAIL $bad"
exit 1
