#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_varname_check_selftest.sh - tools/mp2s_varname_check.sh without a board.
#
#   tools/mp2s_varname_check_selftest.sh
#
# Fixtures: a clean trio (exit 0), a collision with ST's env and with the deny
# list (exit 1), a set mismatch both ways (exit 1). Then the real tree: the
# design has the script set boot_prefixes and save it, and the check's deny
# list and the ST fixture both name it, so the real tree is EXPECTED to report
# exactly those two boot_prefixes findings (and no other) until the design or
# the deny list is decided; a different result fails this selftest.
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk=$here/mp2s_varname_check.sh
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/mp2s_vn.XXXXXX") || exit 2
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

printf 'bootcmd=run bootcmd_stm32mp\nboot_targets=mmc0\nbootdelay=1\n' >"$tmp/st.txt"
cat >"$tmp/note.md" <<'EOF'
## 3. The counted default

| variable | written by | meaning |
|---|---|---|
| `mp2s_left` | runner | count |
| `mp2s_ok` | script | check |

## 7. The interface contract

### Variables

Section 3's table.
EOF
cat >"$tmp/s.cmd" <<'EOF'
# comment: setenv bootcount 1
setenv mp2s_left 2
setexpr mp2s_left ${mp2s_left} - 1
if size mmc 0:1 /x; then setenv mp2s_ok 1; fi
echo setenv bootcmd x
EOF
run clean 0 'ok (2 names' -s "$tmp/s.cmd" -n "$tmp/note.md" -f "$tmp/st.txt"

# collision with the deny list and with ST's env
sed 's/^echo.*/setenv bootcount 0\nenv set altbootcmd x/' "$tmp/s.cmd" >"$tmp/s_col.cmd"
run deny-collision 1 'deny list: bootcount' -s "$tmp/s_col.cmd" -n "$tmp/note.md" -f "$tmp/st.txt"
run deny-collision-altbootcmd 1 'deny list: altbootcmd' -s "$tmp/s_col.cmd" -n "$tmp/note.md" -f "$tmp/st.txt"
printf 'mp2s_ok=1\n' >"$tmp/st_col.txt"
run st-collision 1 'fixture): mp2s_ok' -s "$tmp/s.cmd" -n "$tmp/note.md" -f "$tmp/st_col.txt"

# set mismatch, both directions
{ cat "$tmp/s.cmd"; echo 'setenv mp2s_extra 1'; } >"$tmp/s_more.cmd"
run script-only 1 'not listed in the note: mp2s_extra' -s "$tmp/s_more.cmd" -n "$tmp/note.md" -f "$tmp/st.txt"
sed 's/^| `mp2s_ok`.*/&\n| `mp2s_ghost` | script | none |/' "$tmp/note.md" >"$tmp/note_more.md"
run note-only 1 'never set by the script: mp2s_ghost' -s "$tmp/s.cmd" -n "$tmp/note_more.md" -f "$tmp/st.txt"
run missing-input 2 '' -s "$tmp/none.cmd" -n "$tmp/note.md" -f "$tmp/st.txt"

# the real tree: the two boot_prefixes findings and nothing else
out=$(bash "$chk" 2>&1); rc=$?
want=$'CONFLICT: collides with ST\'s default env (fixture): boot_prefixes\nCONFLICT: collides with the deny list: boot_prefixes'
if [ "$rc" = 1 ] && [ "$out" = "$want" ]; then
	ok "real tree: expected boot_prefixes findings (rc 1)"
else
	bad "real tree: rc $rc, output: $out"
fi

[ "$fail" = 0 ] && echo "mp2s_varname_check_selftest: all ok"
exit "$fail"
