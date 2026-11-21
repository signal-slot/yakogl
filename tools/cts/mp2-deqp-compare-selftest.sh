#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/cts/mp2-deqp-compare-selftest.sh
#
# Host-only check of tools/cts/mp2-deqp-compare.py on synthetic inputs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-deqp-compare.py
D=$(mktemp -d "$TMPDIR/mp2-dc.XXXXXX" 2>/dev/null || mktemp -d) || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
chk() { l=$1; shift; if "$@"; then echo "  ok    $l"; else echo "  FAIL  $l"; fail=1; fi; }
P=dEQP-GLES2.functional
H=$(printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names')
run() { python3 -I "$TOOL" --imx8mp "$D/imx.tsv" "$@" 2>&1; }
has() { printf '%s\n' "$1" | grep -qx -- "$2"; }
# the line after the heading matching $2
first() { printf '%s\n' "$1" | grep -m1 -A1 -- "$2" | tail -1 | grep -qx -- "$3"; }

# i.MX8MP: a, b fail
printf '%s\n0\tf\t9\t7\t2\t0\t0\t0\t%s.a;%s.b\n' "$H" $P $P >"$D/imx.tsv"
# DK log: b Fail, c Crash, a Pass, d NotSupported; e Fail then Pass; noise, CR, stamp
printf '12:00:01 deqp: Pass %s.a\r\ndeqp: Fail %s.b\ndeqp: Crash %s.c\ndeqp: NotSupported %s.d\nnoise\ndeqp: Fail %s.e\ndeqp: Pass %s.e\n' \
  $P $P $P $P $P $P >"$D/dk.log"

out=$(run "$D/dk.log"); rc=$?
chk "log: summary" has "$out" 'mp2-deqp-compare: dk-only 1, imx8mp-only 1, common 1'
chk "log: dk-only is c" first "$out" 'DK-only' "$P.c"
chk "log: imx-only is a" first "$out" 'i.MX8MP-only' "$P.a"
chk "log: common is b" first "$out" '== common' "$P.b"
chk "log: last status wins (e not listed)" test -z "$(printf '%s\n' "$out" | grep "$P.e")"
chk "log: exit 0 without --strict" test $rc -eq 0
run --strict "$D/dk.log" >/dev/null
chk "--strict exits 1 on dk-only" test $? -eq 1

# tsv form, nothing new
printf '%s\n0\tf\t9\t8\t1\t0\t0\t0\t%s.a\n' "$H" $P >"$D/dk.tsv"
out=$(run --strict "$D/dk.tsv"); rc=$?
chk "tsv: no dk-only, --strict exit 0" test $rc -eq 0
chk "tsv: summary" has "$out" 'mp2-deqp-compare: dk-only 0, imx8mp-only 1, common 1'

# plain list: x fails, a passes, bare b is a failing name
printf 'Fail %s.x\nPass %s.a\n%s.b\n' $P $P $P >"$D/dk.txt"
out=$(run "$D/dk.txt")
chk "plain list" has "$out" 'mp2-deqp-compare: dk-only 1, imx8mp-only 1, common 1'

run "$D/nope" >/dev/null
chk "missing file exits 2" test $? -eq 2
h=$(python3 -I "$TOOL" --help)
chk "--help names the log form" sh -c 'printf "%s" "$1" | grep -q "deqp: <Status>"' _ "$h"
chk "--help names results.tsv and its source" sh -c 'printf "%s" "$1" | grep -q "results.tsv" && printf "%s" "$1" | grep -q gc_result' _ "$h"
chk "default i.MX8MP baseline loads" python3 -I "$TOOL" --quiet "$D/dk.tsv"

[ $fail -eq 0 ] && echo "mp2-deqp-compare selftest: ok"
exit $fail
