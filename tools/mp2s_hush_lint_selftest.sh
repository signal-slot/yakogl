#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of tools/mp2s_hush_lint.py: bad fixtures, each of which must fail
# with its own message, and the real driver/tools/mp2-standalone/mp2-standalone.cmd,
# which must pass.  No board, no QEMU.  Needs python3 only.
set -u
here=$(cd "$(dirname "$0")" && pwd)
lint="$here/mp2s_hush_lint.py"
real="$here/../driver/tools/mp2-standalone/mp2-standalone.cmd"
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-hush-lint.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT

pass=0
fail=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }

# expect_fail NAME PATTERN FILE [lint args]: exit 1 and PATTERN in the output
expect_fail() {
	local name=$1 pat=$2 f=$3 out rc
	shift 3
	out=$(python3 -I "$lint" "$@" "$f" 2>&1)
	rc=$?
	if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q -- "$pat"; then
		ok "$name"
	else
		bad "$name (rc $rc, wanted 1 and /$pat/)"
		printf '%s\n' "$out" | sed 's/^/     /'
	fi
}

# fixture NAME BODY: write a file whose last line has a newline
fixture() { printf '%s\n' "$2" > "$work/$1.cmd"; }

# A good base: the counted path in miniature.
good='echo "mp2s: undo"
if test -z "${mp2s_left}"; then
	exit
fi
if test "${mp2s_left}" -gt 0; then
	setexpr mp2s_left ${mp2s_left} - 1
	if saveenv; then
		echo ok
	fi
fi'
fixture good "$good"
out=$(python3 -I "$lint" "$work/good.cmd" 2>&1) && ok "good base passes" ||
	{ bad "good base passes"; echo "$out"; }

fixture unbalanced_if 'if test 1 = 1; then
	echo a'
expect_fail "if without fi" "if is never closed" "$work/unbalanced_if.cmd"
fixture stray_fi 'echo a
fi'
expect_fail "fi without if" "fi without a matching if" "$work/stray_fi.cmd"
fixture loop_open 'for i in a b; do
	echo ${i}'
expect_fail "for without done" "for is never closed" "$work/loop_open.cmd"
fixture stray_done 'echo a
done'
expect_fail "done without for" "done without a matching" "$work/stray_done.cmd"
fixture arith 'x=$((1 + 2))'
expect_fail "\$(( ))" 'bash-only \$((' "$work/arith.cmd"
fixture dbl 'if [[ -n x ]]; then echo a; fi'
expect_fail "[[ ]]" 'bash-only \[\[' "$work/dbl.cmd"
fixture func 'f() { echo a; }'
expect_fail "function ()" "function definition" "$work/func.cmd"
fixture func2 'function f { echo a; }'
expect_fail "function keyword" "function definition" "$work/func2.cmd"
fixture loc 'local x=1'
expect_fail "local" "'local' is not available" "$work/loc.cmd"
fixture arr 'a=(1 2 3)'
expect_fail "array" "array assignment" "$work/arr.cmd"
fixture heredoc 'cat <<EOF
x
EOF'
expect_fail "here-doc" "here-document" "$work/heredoc.cmd"
fixture expr 'expr 1 + 2'
expect_fail "expr" "'expr' is not available" "$work/expr.cmd"
fixture pexp 'echo ${x%%.*}'
expect_fail "\${x%}" 'parameter expansion' "$work/pexp.cmd"
fixture bootcmd 'setenv bootcmd run foo'
expect_fail "setenv bootcmd" "setenv of bootcmd" "$work/bootcmd.cmd"
fixture bootcount 'setenv bootcount 0'
expect_fail "setenv bootcount" "setenv of bootcount" "$work/bootcount.cmd"
fixture altbootcmd 'setenv altbootcmd run foo'
expect_fail "setenv altbootcmd" "setenv of altbootcmd" "$work/altbootcmd.cmd"
fixture save 'setenv mp2s_load 0xa0000000
saveenv'
expect_fail "saveenv outside the counted path" "saveenv outside" "$work/save.cmd"
fixture save_first 'saveenv'
expect_fail "saveenv first" "saveenv outside" "$work/save_first.cmd"
# A word in a string or a comment is not a command.
fixture words 'echo "mp2s: undo: setenv bootcmd; saveenv; boot"
# setenv bootcmd x; saveenv; [[ ]]
echo '"'"'$(( ))'"'"''
out=$(python3 -I "$lint" "$work/words.cmd" 2>&1) && ok "quoted and comment text ignored" ||
	{ bad "quoted and comment text ignored"; echo "$out"; }

long=$(printf 'x%.0s' $(seq 1 1100))
fixture longline "echo $long"
expect_fail "line over CBSIZE" "CONFIG_SYS_CBSIZE is 1024" "$work/longline.cmd"
# continuation lines are joined before the length check
{ printf 'echo %s\\\n' "$(printf 'y%.0s' $(seq 1 600))"
  printf '%s\n' "$(printf 'y%.0s' $(seq 1 600))"; } > "$work/joined.cmd"
expect_fail "joined lines over CBSIZE" "line is 1205 characters" "$work/joined.cmd"
fixture short 'echo abcdefghijklmnopqrstuvwxyz'
expect_fail "--cbsize 20" "CONFIG_SYS_CBSIZE is 20" "$work/short.cmd" --cbsize 20
printf 'CONFIG_SYS_CBSIZE=16\n' > "$work/cb.config"
expect_fail "CBSIZE from the config" "CONFIG_SYS_CBSIZE is 16" "$work/short.cmd" \
	--config "$work/cb.config"

printf 'echo a' > "$work/nonl.cmd"
expect_fail "no trailing newline" "does not end with a newline" "$work/nonl.cmd"
printf 'echo a\r\n' > "$work/cr.cmd"
expect_fail "CR" "CR in file" "$work/cr.cmd"

# The real script.  CBSIZE: qemu/mp2-standalone.config sets none, so the
# stm32mp25 default 1024 is used and the lint says so.
out=$(python3 -I "$lint" "$real" 2>&1) && rc=0 || rc=$?
if [ "$rc" -eq 0 ]; then
	ok "mp2-standalone.cmd is clean"
else
	bad "mp2-standalone.cmd is clean (rc $rc)"
	printf '%s\n' "$out" | sed 's/^/     /'
fi
if printf '%s\n' "$out" | grep -q 'CONFIG_SYS_CBSIZE 1024 (default 1024 (stm32mp25)'; then
	ok "CBSIZE 1024 reported with its source"
else
	bad "CBSIZE report"
fi

echo "$pass ok, $fail failed"
[ "$fail" -eq 0 ]
