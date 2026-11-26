#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/cts/t6-deqp-images.sh --plan: every row of the
# repository's own mali-gles2-tree.txt resolves, the environment is checked
# by name (a set value must never be printed), unknown rows, missing names,
# a bad DEQP_MODULE and a bad table are refused, the commands and the output
# paths of the coming build are printed, and nothing is built. Needs no
# toolchain, no west workspace and no board; the labels are read from the
# table, so a row moved or renamed here changes nothing.
# tools/check_selftests.py gives this file its row in the gate.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$HERE/mali-gles2-tree.txt
S=$HERE/t6-deqp-images.sh
W=$(mktemp -d "${TMPDIR:-/tmp}/t6plan.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
unset DEQP_MODULE DEQP_STAGE1
# Values no test may ever see on stdout or stderr of the script under test.
EWS=planws7c4e ESDK=plansdk7c4e EWORK=planwork7c4e
export ZEPHYR_WS=$EWS ZEPHYR_SDK_INSTALL_DIR=$ESDK DEQP_WORK=$EWORK

# every "# image <LABEL>:" label, as the builder reads it
labels=$(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$T")
[ -n "$labels" ] || { echo "no image row in $T"; exit 1; }
first=${labels%%$'\n'*}

out=$("$S" --plan "$T" "$W/img" "$first" 2>"$W/err"); rc=$?
if [ $rc = 0 ]; then ok "a known row: exit 0"; else bad "known row: rc $rc: $(cat "$W/err")"; fi
if grep -q "^$first: filter " <<<"$out" && grep -q "cmake .*-B $W/img/$first -DBOARD=nanopc_t6 " <<<"$out" &&
	grep -q "ninja -C $W/img/$first\$" <<<"$out" &&
	grep -q "$W/img/$first/zephyr/zephyr.bin" <<<"$out" &&
	grep -q "$W/img/$first.built" <<<"$out" && grep -q "$W/img/$first.log" <<<"$out" &&
	grep -q -- "-DCONFIG_DEQP_CASE=" <<<"$out"; then
	ok "the cmake and ninja commands and the image, .built and log paths are printed"
else bad "plan output: $(head -4 <<<"$out")"; fi
if grep -q "$EWS\|$ESDK\|$EWORK" <<<"$out$(cat "$W/err")"; then
	bad "a value of the environment was printed"
else ok "the environment is named, never valued"; fi
if [ ! -e "$W/img" ]; then ok "nothing is built"; else bad "--plan created $W/img"; fi

all=$("$S" --plan "$T" "$W/img" 2>/dev/null); rc=$?
if [ $rc = 0 ]; then ok "no LABEL: exit 0 (all rows)"; else bad "no LABEL: rc $rc"; fi
rowsok=1
for l in $labels; do
	c=$(grep -c "^$l: filter " <<<"$all")
	[ "$c" = 1 ] || { bad "row $l: $c plan lines, want 1"; rowsok=0; }
done
[ $rowsok = 1 ] && ok "every label of $T names exactly one plan row ($(printf '%s\n' "$labels" | grep -c .) rows)"

"$S" --plan "$T" "$W/img" NOSUCHROW >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "unknown row NOSUCHROW" "$W/err"; then
	ok "an unknown row: refused, named"
else bad "unknown row: rc $rc: $(cat "$W/err")"; fi
"$S" --plan "$T" "$W/img" "$first" NOSUCHROW >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "unknown row NOSUCHROW" "$W/err"; then
	ok "an unknown row beside a known one: refused, named"
else bad "mixed rows: rc $rc: $(cat "$W/err")"; fi

for v in ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK; do
	env -u "$v" "$S" --plan "$T" "$W/img" "$first" >"$W/o" 2>"$W/err"; rc=$?
	if [ $rc != 0 ] && grep -q "$v is not set" "$W/err" &&
		! grep -q "$EWS\|$ESDK\|$EWORK" "$W/err"; then
		ok "a missing $v: refused, named, unvalued"
	else bad "missing $v: rc $rc: $(cat "$W/err")"; fi
done
env -u DEQP_WORK DEQP_STAGE1=$W/stage1 "$S" --plan "$T" "$W/img" "$first" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "DEQP_WORK is not set" "$W/err" && grep -q "DEQP_STAGE1 is set" "$W/err"; then
	ok "DEQP_STAGE1 alone: refused, and the message points at DEQP_WORK"
else bad "DEQP_STAGE1 alone: rc $rc: $(cat "$W/err")"; fi

out=$(DEQP_MODULE=gles3 "$S" --plan "$T" "$W/img" "$first" 2>"$W/err"); rc=$?
if [ $rc = 0 ] && grep -q -- "-DDEQP_MODULE=gles3" <<<"$out" && grep -q -- "--gles3" <<<"$out"; then
	ok "DEQP_MODULE=gles3: its cmake arguments and the --gles3 check are in the plan"
else bad "DEQP_MODULE=gles3: rc $rc: $(head -3 <<<"$out") $(cat "$W/err")"; fi
DEQP_MODULE=gl3 "$S" --plan "$T" "$W/img" "$first" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "DEQP_MODULE must be gles2 or gles3" "$W/err"; then
	ok "a bad DEQP_MODULE: refused"
else bad "bad DEQP_MODULE: rc $rc: $(cat "$W/err")"; fi

# a texture-2048.conf row of one's own: the marker's argument joins that row's
# cmake line only
printf '# image A1: x\ndEQP-GLES2.a.*\n# image B7: y (with texture-2048.conf)\ndEQP-GLES2.b.*\n' > "$W/tex.txt"
out=$("$S" --plan "$W/tex.txt" "$W/img" 2>"$W/err"); rc=$?
if [ $rc = 0 ] && grep -q -- "-DEXTRA_CONF_FILE=texture-2048.conf" <<<"$out" &&
	[ "$(grep -c 'texture-2048' <<<"$out")" = 1 ] &&
	grep -q "^A1: filter dEQP-GLES2.a.\*$" <<<"$out"; then
	ok "a texture-2048.conf row: the argument on its own cmake line"
else bad "texture row: rc $rc: $(head -3 <<<"$out")"; fi

"$S" --plan "$W/none.txt" "$W/img" >"$W/o" 2>"$W/err"; rc=$?
[ $rc != 0 ] && ok "a missing table: refused" || bad "missing table: rc $rc"
printf '# image A1: x\ndEQP-GLES2.a.*\ndEQP-GLES2.b.*\n' > "$W/nolabel.txt"
"$S" --plan "$W/nolabel.txt" "$W/img" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "filter line without a label" "$W/err"; then
	ok "a filter line without a label: refused, the line named"
else bad "no-label line: rc $rc: $(cat "$W/err")"; fi
"$S" --plan >"$W/o" 2>"$W/err"; rc=$?
[ $rc != 0 ] && ok "--plan without arguments: refused with the usage" || bad "--plan alone: rc $rc"

# --dry-run is the builder's pass-through: rows, no environment, nothing built
out=$(env -u ZEPHYR_WS -u DEQP_WORK "$S" --dry-run "$T" "$W/img2" 2>"$W/err"); rc=$?
if [ $rc = 0 ] && [ "$(grep -c '^[A-Za-z0-9]* dEQP-GLES2' <<<"$out")" = "$(printf '%s\n' "$labels" | grep -c .)" ] &&
	[ ! -e "$W/img2" ]; then
	ok "--dry-run still names every row with its filter, no environment, nothing built"
else bad "--dry-run: rc $rc: $(cat "$W/err")"; fi

[ $fail = 0 ] && echo "t6-deqp-images-selftest: all ok"
exit $fail
