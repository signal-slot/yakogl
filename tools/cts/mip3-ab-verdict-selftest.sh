#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of mip3-ab-verdict.py on the synthetic, redacted record pairs under
# tools/cts/testdata/mip3-ab/: confirmed, kickbound (kicks fall, ms do not),
# picture (the split moves) and missing (a line absent). Host only; no board.
# Exit 0 when every check passes.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/mip3-ab-verdict.py
data=$here/testdata/mip3-ab
work=$(mktemp -d "${TMPDIR:-/var/tmp}/mip3ab.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
run() { python3 -I "$tool" "$data/$1/A" "$data/$1/E" > "$work/out" 2> "$work/err"; echo $?; }
has() { grep -q -e "$1" "$work/$2"; }

rc=$(run confirmed)
if [ "$rc" = 0 ]; then ok "confirmed: exit 0"; else bad "confirmed: exit $rc"; fi
has '^mip3-ab: confirmed$' out && ok "confirmed: last line" || bad "confirmed: last line"
has '^| gl_test 216 `per frame: N kicks` | 40.00 | 21.00 |$' out && ok "table row kicks" || bad "table row kicks"
has '^| record line | A (off) | E (on), measured |$' out && ok "table header" || bad "table header"
has '^case216-ms: ok: .*met by' out && ok "confirmed: margin" || bad "confirmed: margin"
has '^kick-bound: yes' out && ok "confirmed: kick-bound yes" || bad "confirmed: kick-bound"

rc=$(run kickbound)
if [ "$rc" = 1 ]; then ok "kickbound: exit 1"; else bad "kickbound: exit $rc"; fi
has '^mip3-ab: missed (case216-ms, terrain-ms)$' out && ok "kickbound: names the misses" || bad "kickbound: names"
has '^kick-bound: no, the kicks fell and the ms did not' out && ok "kickbound: line" || bad "kickbound: line"
has '^case216-kicks-pass: ok' out && ok "kickbound: kicks still ok" || bad "kickbound: kicks"

rc=$(run picture)
if [ "$rc" = 1 ]; then ok "picture: exit 1"; else bad "picture: exit $rc"; fi
has '^mip3-ab: missed (pictures)$' out && ok "picture: names it" || bad "picture: names"
has '^pictures: miss: split moved' out && ok "picture: split line" || bad "picture: split line"

rc=$(run missing)
if [ "$rc" = 2 ]; then ok "missing: exit 2"; else bad "missing: exit $rc"; fi
has "no case 216 'per frame" err && ok "missing: names the line" || bad "missing: names"
[ ! -s "$work/out" ] && ok "missing: no verdict printed" || bad "missing: printed"

python3 -I "$tool" "$data/confirmed/A" "$work/nothere" > /dev/null 2>&1
[ $? = 2 ] && ok "unreadable record: exit 2" || bad "unreadable record"
python3 -I "$tool" --c 0.5 "$data/confirmed/A" "$data/confirmed/E" > "$work/out" 2>&1
has '^c = 0.50 ms (--c)$' out && ok "--c overrides" || bad "--c"

# fixtures carry no address, path or host name
if grep -rEq '([0-9]{1,3}\.){3}[0-9]{1,3}|/home/|/dev/tty|/Users/' "$data"; then bad "fixtures redacted"; else ok "fixtures redacted"; fi

exit $fail
