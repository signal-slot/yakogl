#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_contract_check_selftest.sh - tools/mp2s_contract_check.sh without a board.
#
#   tools/mp2s_contract_check_selftest.sh
#
# Fixtures: a clean trio (exit 0), a variable missing from the note, a console
# line missing from the script, a stale (pending) marker, an absent record
# check (skipped, exit 0), a record-check regex that matches nothing (exit 1).
# Then one run on the real tree that prints its findings and asserts nothing
# about them: the note belongs to other items.
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk=$here/mp2s_contract_check.sh
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/mp2s_cc.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

# run <name> <want-rc> <grep-pattern-or-empty> args...
run() {
	local name=$1 want=$2 pat=$3 out rc
	shift 3
	out=$(bash "$chk" "$@" 2>&1); rc=$?
	if [ "$rc" != "$want" ]; then bad "$name: exit $rc, want $want: $out"; return; fi
	if [ -n "$pat" ] && ! printf '%s\n' "$out" | grep -q -- "$pat"; then
		bad "$name: output lacks '$pat': $out"; return
	fi
	ok "$name"
}

mkdir -p "$tmp/root/tools"
cat >"$tmp/s.cmd" <<'EOF'
# comment: echo "mp2s: not printed"
echo "mp2s: undo: setenv mp2s_left 0; saveenv; boot"
if test "${mp2s_left}" = forever; then
	echo "mp2s: default /x/default.bin, forever"
else
	setexpr mp2s_left ${mp2s_left} - 1
	echo "mp2s: default /x/default.bin, boots left ${mp2s_left}"
fi
if test -z "${devnum}"; then
	echo "mp2s: refused: devnum is not set, booting Linux"
fi
EOF
cat >"$tmp/n.md" <<'EOF'
## 7. The interface contract

The variables are `mp2s_left` and `mp2s_ver`; the tool `mp2s_guard.py` is not
one, and `tools/mp2s_foo.py` neither.

### Console lines

```
mp2s: undo: setenv mp2s_left 0; saveenv; boot
mp2s: default /x/default.bin, boots left <n>      <- after the save
mp2s: default /x/default.bin, forever
mp2s: refused: <why>, booting Linux
```
EOF
printf 'setenv mp2s_ver 1\n' >>"$tmp/s.cmd"
args=(--script "$tmp/s.cmd" --note "$tmp/n.md" --record-check "$tmp/none.py" --root "$tmp/root")
run clean 0 'ok (' "${args[@]}"
run absent-skip 0 'record check absent, skipped' "${args[@]}"

# a variable the note does not name, and one only the note names
sed 's/ and `mp2s_ver`//' "$tmp/n.md" >"$tmp/n1.md"
run var-missing-from-note 1 'CONTRACT: variable mp2s_ver in s.cmd missing from n1.md' \
	--script "$tmp/s.cmd" --note "$tmp/n1.md" --root "$tmp/root"
sed 's/`mp2s_left` and/`mp2s_left`, `mp2s_installed` and/' "$tmp/n.md" >"$tmp/n2.md"
run var-missing-from-script 1 'CONTRACT: variable mp2s_installed in n2.md missing from s.cmd' \
	--script "$tmp/s.cmd" --note "$tmp/n2.md" --root "$tmp/root"

# a console line the script does not print / the note does not quote
sed 's/^mp2s: default \/x\/default.bin, forever/mp2s: default forever, twice/' "$tmp/n.md" >"$tmp/n3.md"
run line-missing-from-script 1 "console line 'mp2s: default forever, twice' in n3.md missing from s.cmd" \
	--script "$tmp/s.cmd" --note "$tmp/n3.md" --root "$tmp/root"
run line-missing-from-note 1 "in s.cmd missing from n3.md" \
	--script "$tmp/s.cmd" --note "$tmp/n3.md" --root "$tmp/root"

# a (pending) line whose tool exists
printf 'x\n' >"$tmp/root/tools/mp2s_foo.py"
{ cat "$tmp/n.md"; echo; echo 'tools/mp2s_foo.py (pending)   # merged'; echo 'tools/mp2s_gone.py (pending)'; } >"$tmp/n4.md"
run stale-pending 1 'STALE-PENDING: tools/mp2s_foo.py' \
	--script "$tmp/s.cmd" --note "$tmp/n4.md" --root "$tmp/root"
out=$(bash "$chk" --script "$tmp/s.cmd" --note "$tmp/n4.md" --root "$tmp/root" 2>&1)
if printf '%s\n' "$out" | grep -q 'mp2s_gone'; then bad "stale-pending: a missing tool was reported"; else ok "stale-pending: a missing tool is not"; fi

# a record check: a matching one is clean, a regex that matches nothing fails
cat >"$tmp/r_ok.py" <<'EOF'
import re
BOOTS = re.compile(r"mp2s: default \S+, boots left (\d+)")
FOREVER = "mp2s: default /x/default.bin, forever"
LINUX = re.compile(r"Starting kernel")
EOF
run record-ok 0 'ok (' --script "$tmp/s.cmd" --note "$tmp/n.md" --record-check "$tmp/r_ok.py" --root "$tmp/root"
cat >"$tmp/r_bad.py" <<'EOF'
import re
BOOTS = re.compile(r"mp2s: default \S+, boots left (\d+)")
GONE = re.compile(r"mp2s: image started, boots remaining (\d+)")
EOF
run record-nomatch 1 'record-check pattern .mp2s: image started' \
	--script "$tmp/s.cmd" --note "$tmp/n.md" --record-check "$tmp/r_bad.py" --root "$tmp/root"

run usage 2 '' --bogus

# the real tree: print the findings, assert nothing
echo "--- real tree (not asserted)"
bash "$chk" 2>&1 | sed 's/^/    /'
echo "    exit ${PIPESTATUS[0]}"

[ $fail = 0 ] && echo "mp2s_contract_check_selftest: ok" || echo "mp2s_contract_check_selftest: FAILED"
exit $fail
