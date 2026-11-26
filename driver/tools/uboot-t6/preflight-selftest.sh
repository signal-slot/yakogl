#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# preflight-selftest.sh - preflight.sh on fake image directories (the single
# layout and tools/cts/t6-deqp-images.sh's row layout) and a fake repository
# under $TMPDIR. No board, no network.
#
#   bash driver/tools/uboot-t6/preflight-selftest.sh
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PF=$HERE/preflight.sh
W=$(mktemp -d "${TMPDIR:-/tmp}/preflight-st.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fails=0
check() { if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }

R=$W/repo
git init -q "$R"
git -C "$R" config user.email t@t
git -C "$R" config user.name t
echo 'records/*' >"$R/.gitignore"
git -C "$R" add .gitignore
git -C "$R" commit -qm one
C1=$(git -C "$R" rev-parse HEAD)
git -C "$R" update-ref refs/remotes/origin/main "$C1"
echo x >"$R/x"
git -C "$R" add x
git -C "$R" commit -qm two
C2=$(git -C "$R" rev-parse HEAD) # not on origin/main

mkimg() { # dir commit
	mkdir -p "$1"
	printf 'image-a' >"$1/t6_a.bin"
	printf 'image-b' >"$1/t6_b.bin"
	(cd "$1" && sha256sum t6_a.bin t6_b.bin >SHA256SUMS)
	[ -z "$2" ] || echo "$2" >"$1/COMMIT"
}
mkimg "$W/good" "$C1"
mkimg "$W/unreach" "$C2"
mkimg "$W/nocommit" ""
mkimg "$W/nosha" "$C1"
sed -i '/t6_b/d' "$W/nosha/SHA256SUMS"
mkimg "$W/badsha" "$C1"
printf 'tampered' >"$W/badsha/t6_a.bin"
mkdir "$W/empty"
# row image directory (tools/cts/t6-deqp-images.sh's <img-dir>: the label is a
# cmake build directory, the image is <LABEL>/zephyr/zephyr.bin, and
# <LABEL>.built files sit at the top). sums-where: top = one SHA256SUMS at the
# top naming the whole relative path, per = a SHA256SUMS under each label.
mkrows() { # dir commit sums-where
	local d=$1 c=$2 where=${3:-top} ha hb
	mkdir -p "$d/A00/zephyr" "$d/B01/zephyr"
	printf 'row-a' >"$d/A00/zephyr/zephyr.bin"
	printf 'row-b' >"$d/B01/zephyr/zephyr.bin"
	if [ "$where" = per ]; then
		(cd "$d/A00" && sha256sum zephyr/zephyr.bin >SHA256SUMS)
		(cd "$d/B01" && sha256sum zephyr/zephyr.bin >SHA256SUMS)
	else
		(cd "$d" && sha256sum A00/zephyr/zephyr.bin B01/zephyr/zephyr.bin >SHA256SUMS)
	fi
	ha=$(sha256sum "$d/A00/zephyr/zephyr.bin" | cut -d' ' -f1)
	hb=$(sha256sum "$d/B01/zephyr/zephyr.bin" | cut -d' ' -f1)
	echo "A00 rc=0 sha256=$ha build=x" >"$d/A00.built"
	echo "B01 rc=0 sha256=$hb build=x" >"$d/B01.built"
	[ -z "$c" ] || echo "$c" >"$d/COMMIT"
}
mkrows "$W/rows" "$C1"
mkrows "$W/rows_per" "$C1" per
mkrows "$W/rows_nosums" "$C1"; rm "$W/rows_nosums/SHA256SUMS"
mkrows "$W/rows_noentry" "$C1"; sed -i '/B01/d' "$W/rows_noentry/SHA256SUMS"
mkrows "$W/rows_badsha" "$C1"; printf 'tampered' >"$W/rows_badsha/A00/zephyr/zephyr.bin"
mkrows "$W/rows_nocommit" ""
mkrows "$W/rows_nobin" "$C1"; rm "$W/rows_nobin/B01/zephyr/zephyr.bin"
mkrows "$W/rows_stray" "$C1"; echo stray >"$W/rows_stray/not_an_image.bin"

S1=secret-uart-value-1
S2=secret-hand-value-2
run() { # dir [VAR=value...]; sets out, rc
	local d=$1
	shift
	out=$(env -i PATH="$PATH" HOME="$W" PREFLIGHT_REPO="$R" "$@" bash "$PF" "$d" 2>&1)
	rc=$?
}
has() { [ "$rc" != 0 ] && grep -q "$1" <<<"$out"; }
ALL=

run "$W/good" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
check "good dir: exit 0 (rc $rc)" "$rc"
grep -q 'preflight: all ok' <<<"$out"; check "good dir: all ok" $?
grep -q 'ok   e: next, run: bash .*/uboot-t6/console-check.sh' <<<"$out"; check "console-check.sh in this tree: command printed" $?

run "$W/good" T6_UART=$S1; ALL="$ALL$out"
has 'FAIL a: not set: T6_HAND'; check "T6_HAND unset: FAIL a" $?
run "$W/good"; ALL="$ALL$out"
has 'FAIL a: not set: T6_UART T6_HAND'; check "both unset: FAIL a" $?
run "$W/unreach" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: build commit .* not reachable'; check "commit off origin/main: FAIL c" $?
run "$W/nocommit" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: no build commit'; check "no COMMIT: FAIL c" $?
run "$W/nosha" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: t6_b.bin has no sha256'; check "image without sha line: FAIL c" $?
run "$W/badsha" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: t6_a.bin sha256 differs'; check "image with wrong sha: FAIL c" $?
run "$W/empty" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: no image'; check "empty dir: FAIL c" $?
# row layout (tools/cts/t6-deqp-images.sh's <img-dir>)
run "$W/rows" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
check "row dir, SHA256SUMS at the top: exit 0 (rc $rc)" "$rc"
[ "$(grep -c 'ok   c: .*/zephyr/zephyr.bin exists, sha256 matches' <<<"$out")" = 2 ]; check "row dir: both label images checked (2 ok lines)" $?
grep -q 'preflight: all ok' <<<"$out"; check "row dir: all ok" $?
run "$W/rows_per" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
check "row dir, SHA256SUMS per label: exit 0 (rc $rc)" "$rc"
grep -q 'ok   c: B01/zephyr/zephyr.bin exists' <<<"$out"; check "row dir, per-label SHA256SUMS: the images checked" $?
run "$W/rows_nosums" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
[ "$rc" != 0 ]; check "row dir without SHA256SUMS: exit 1 (rc $rc)" $?
[ "$(grep -c 'no SHA256SUMS at the top or in' <<<"$out")" = 2 ]; check "row dir without SHA256SUMS: each label says where it looked" $?
run "$W/rows_noentry" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: B01/zephyr/zephyr.bin has no sha256 in SHA256SUMS'; check "row dir missing one sha line: FAIL c names it" $?
grep -q 'ok   c: A00/zephyr/zephyr.bin exists' <<<"$out"; check "row dir missing one sha line: the other image still ok" $?
run "$W/rows_badsha" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: A00/zephyr/zephyr.bin sha256 differs from SHA256SUMS'; check "row dir with a wrong sha: FAIL c names it" $?
run "$W/rows_nocommit" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: no build commit'; check "row dir without COMMIT: FAIL c" $?
run "$W/rows_nobin" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: B01/zephyr/zephyr.bin is missing'; check "row dir whose label has no zephyr.bin: FAIL c names it" $?
run "$W/rows_stray" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL c: top-level file of a row image directory, not a <LABEL>.built, SHA256SUMS or COMMIT: not_an_image.bin'; check "row dir with a stray top-level file: FAIL c names it" $?
run "$W/none" T6_UART=$S1 T6_HAND=$S2
[ "$rc" = 2 ]; check "missing dir: usage exit 2 (rc $rc)" $?

# d: a repository that does not ignore records/
: >"$R/.gitignore"
run "$W/good" T6_UART=$S1 T6_HAND=$S2; ALL="$ALL$out"
has 'FAIL d: records/ is not gitignored'; check "records/ not ignored: FAIL d" $?
echo 'records/*' >"$R/.gitignore"

# b: a python3 that cannot import anything
mkdir "$W/bin"
printf '#!/bin/sh\nexit 1\n' >"$W/bin/python3"
chmod +x "$W/bin/python3"
out=$(env -i PATH="$W/bin:$PATH" HOME="$W" PREFLIGHT_REPO="$R" T6_UART=$S1 T6_HAND=$S2 bash "$PF" "$W/good" 2>&1)
rc=$?
ALL="$ALL$out"
has 'FAIL b: python3 cannot import'; check "python3 import fails: FAIL b" $?

# e: in a copy of preflight.sh's directory (this tree has console-check.sh):
# without console-check.sh nothing is printed; with an executable one the
# command is printed and the script is not run
F=$W/fake
mkdir -p "$F/driver/tools/uboot-t6" "$F/driver/tools/kexec-zephyr"
cp "$PF" "$HERE/prompt-go.py" "$F/driver/tools/uboot-t6/"
cp "$HERE/../kexec-zephyr/t6-uart-rfc2217.py" "$F/driver/tools/kexec-zephyr/"
run_copy() {
	out=$(env -i PATH="$PATH" HOME="$W" PREFLIGHT_REPO="$R" T6_UART=$S1 T6_HAND=$S2 bash "$F/driver/tools/uboot-t6/preflight.sh" "$W/good" 2>&1)
	ALL="$ALL$out"
}
run_copy
! grep -q 'run: bash' <<<"$out"; check "console-check.sh absent: no command printed" $?
grep -q 'ok   e: console-check.sh not present' <<<"$out"; check "console-check.sh absent: said so" $?
printf '#!/bin/sh\ntouch %s/ran\n' "$W" >"$F/driver/tools/uboot-t6/console-check.sh"
chmod +x "$F/driver/tools/uboot-t6/console-check.sh"
run_copy
grep -q 'ok   e: next, run: bash .*console-check.sh' <<<"$out"; check "console-check.sh executable: command printed" $?
[ ! -e "$W/ran" ]; check "console-check.sh not run" $?

! grep -qF -e "$S1" -e "$S2" <<<"$ALL"; check "no environment value printed" $?

if [ $fails = 0 ]; then echo "preflight-selftest: all ok"; else echo "preflight-selftest: $fails failed"; fi
exit $((fails != 0))
