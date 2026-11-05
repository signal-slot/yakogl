#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-board.sh (usage errors, image choice, argument hand-off to tools/board.sh) and of
# check-imx8mp-gles3-tree.py (passes on the committed table, fails on broken copies of it). The wrapper runs
# from a copy under a temporary ROOT whose tools/board.sh is a stub, so the real board.sh is never called.
# Host only; writes under ${TMPDIR:-$PWD}.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-$PWD}/gles3-wrap-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

# ---- (A) imx8mp-gles3-board.sh under a fake ROOT
root=$tmp/root
mkdir -p "$root/tools/cts"
cp "$here/imx8mp-gles3-board.sh" "$here/imx8mp-gles3-tree.txt" "$root/tools/cts/"
cat > "$root/tools/board.sh" <<'STUB'
#!/bin/bash
echo "ARGS: $*"
echo "IMAGE: ${TREE_IMAGE-<unset>}"
STUB
chmod +x "$root/tools/board.sh" "$root/tools/cts/imx8mp-gles3-board.sh"
w=$root/tools/cts/imx8mp-gles3-board.sh
outdir=$tmp/outdir

usage_err() { # description, 1 if exit 1 is a known mismatch, args...
	local what=$1 known=$2 rc; shift 2
	env -u TREE_IMAGE -u IMX8MP_OUT "$w" "$@" > "$tmp/o" 2> "$tmp/e"; rc=$?
	if [ $rc -eq 2 ] && grep -q 'usage' "$tmp/e"; then ok "$what: exit 2 and usage line"
	elif [ "$known" = 1 ] && [ $rc -eq 1 ] && grep -q 'usage' "$tmp/e"; then
		ok "$what: EXPECTED-MISMATCH: exit 1 (the :? parameter expansion), not 2; usage line printed"
	else bad "$what: exit $rc, stderr: $(cat "$tmp/e")"; fi
}
usage_err "no verb" 1
usage_err "unknown verb" 0 bogus "$outdir"
usage_err "tree without outdir" 1 tree
usage_err "tally without outdir" 1 tally
usage_err "--dry-run alone" 1 --dry-run

# no image under IMX8MP_OUT
mkdir -p "$tmp/empty/es3-none/zephyr"
env -u TREE_IMAGE IMX8MP_OUT="$tmp/empty" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q 'no es3-\* image' "$tmp/e"; then ok "no image: exit 2, 'no es3-* image'"
else bad "no image: exit $rc, stderr: $(cat "$tmp/e")"; fi

# neither TREE_IMAGE nor IMX8MP_OUT
env -u TREE_IMAGE -u IMX8MP_OUT "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -ne 0 ] && grep -q IMX8MP_OUT "$tmp/e"; then ok "no TREE_IMAGE and no IMX8MP_OUT: exit $rc naming IMX8MP_OUT"
else bad "no TREE_IMAGE and no IMX8MP_OUT: exit $rc, stderr: $(cat "$tmp/e")"; fi

# newest image wins
out=$tmp/imgs
mkdir -p "$out/es3-aaaaaaaa/zephyr" "$out/es3-bbbbbbbb/zephyr"
: > "$out/es3-aaaaaaaa/zephyr/zephyr.bin"; : > "$out/es3-bbbbbbbb/zephyr/zephyr.bin"
touch -d '2026-01-01 00:00:00' "$out/es3-aaaaaaaa/zephyr/zephyr.bin"
touch -d '2026-02-01 00:00:00' "$out/es3-bbbbbbbb/zephyr/zephyr.bin"
want_args="ARGS: --dry-run imx8mp tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir"
env -u TREE_IMAGE IMX8MP_OUT="$out" "$w" --dry-run tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "$want_args" "$tmp/o" && grep -qxF "IMAGE: $out/es3-bbbbbbbb/zephyr/zephyr.bin" "$tmp/o"; then
	ok "newest es3-* image (bbbbbbbb) chosen, board.sh got '${want_args#ARGS: }'"
else bad "newest image: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
touch -d '2026-03-01 00:00:00' "$out/es3-aaaaaaaa/zephyr/zephyr.bin"
env -u TREE_IMAGE IMX8MP_OUT="$out" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: imx8mp tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir" "$tmp/o" \
	&& grep -qxF "IMAGE: $out/es3-aaaaaaaa/zephyr/zephyr.bin" "$tmp/o"; then
	ok "after touching aaaaaaaa it is the newest and chosen; no --dry-run without the flag"
else bad "newest image after touch: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# TREE_IMAGE wins over IMX8MP_OUT, and works without IMX8MP_OUT
TREE_IMAGE=$tmp/mine.bin IMX8MP_OUT="$out" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "IMAGE: $tmp/mine.bin" "$tmp/o"; then ok "TREE_IMAGE is used over the es3-* images"
else bad "TREE_IMAGE: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
env -u IMX8MP_OUT TREE_IMAGE="$tmp/mine.bin" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "IMAGE: $tmp/mine.bin" "$tmp/o"; then ok "TREE_IMAGE alone (no IMX8MP_OUT) is enough"
else bad "TREE_IMAGE alone: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# tally
env -u TREE_IMAGE -u IMX8MP_OUT "$w" tally "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: imx8mp tally $outdir" "$tmp/o"; then ok "tally <outdir> -> 'imx8mp tally <outdir>'"
else bad "tally: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# ---- (B) check-imx8mp-gles3-tree.py
chk=$here/check-imx8mp-gles3-tree.py
table=$here/imx8mp-gles3-tree.txt
must=$here/gles3-mustpass.txt
run_chk() { python3 "$chk" -q --table "$1" --mustpass "${2:-$must}" > "$tmp/c" 2>&1; }

if run_chk "$table" && tail -1 "$tmp/c" | grep -qx OK; then ok "checker passes on the committed table"
else bad "checker rejects the committed table: $(tail -3 "$tmp/c")"; fi

expect_fail() { # description, table, mustpass ('' = committed), text the output must hold
	if run_chk "$2" "${3:-$must}"; then bad "$1: checker passed"
	elif ! grep -q -- "$4" "$tmp/c"; then bad "$1: failed, but without '$4': $(tail -3 "$tmp/c")"
	else ok "$1: checker fails ($4)"; fi
}
grep -v '^#' "$table" | grep . > "$tmp/rows.txt"
nrows=$(wc -l < "$tmp/rows.txt")

sed '1d' "$tmp/rows.txt" > "$tmp/dropped.txt"
expect_fail "dropped row" "$tmp/dropped.txt" "" "matched by no entry"

cat "$tmp/rows.txt" <(sed -n '1p' "$tmp/rows.txt") > "$tmp/dupentry.txt"
expect_fail "duplicated row" "$tmp/dupentry.txt" "" "more than one entry"

first=$(sed -n '1p' "$tmp/rows.txt" | cut -d, -f1)
cat "$tmp/rows.txt" <(echo "$first") > "$tmp/duppat.txt"
expect_fail "pattern of row 0 repeated in a new row" "$tmp/duppat.txt" "" "more than one entry"

cat "$tmp/rows.txt" <(echo 'dEQP-GLES3.*') > "$tmp/overlap.txt"
expect_fail "row overlapping the others" "$tmp/overlap.txt" "" "more than one entry"

cat "$tmp/rows.txt" <(echo 'dEQP-GLES3.info.*,') > "$tmp/emptypat.txt"
expect_fail "empty pattern" "$tmp/emptypat.txt" "" "empty pattern"

cat "$tmp/rows.txt" <(head -c 2100 /dev/zero | tr '\0' x; echo) > "$tmp/long.txt"
expect_fail "entry over 2035 characters" "$tmp/long.txt" "" "characters (max 2035)"

cat "$tmp/rows.txt" <(seq 1 256 | sed 's/^/dEQP-GLES3.nosuch.case/') > "$tmp/many.txt"
expect_fail "more than 256 entries" "$tmp/many.txt" "" "(max 256)"

cat "$must" <(echo 'dEQP-GLES3.nosuch.group.case') > "$tmp/must-extra.txt"
expect_fail "mustpass case the table does not cover" "$table" "$tmp/must-extra.txt" "not matched: dEQP-GLES3.nosuch"

sed '1s/\*.*$/zzz/' "$tmp/rows.txt" > "$tmp/narrow.txt"
expect_fail "row narrowed so its cases go unmatched" "$tmp/narrow.txt" "" "matched by no entry"

# comments and blank lines are ignored
{ echo '# comment'; echo; cat "$tmp/rows.txt"; } > "$tmp/commented.txt"
if run_chk "$tmp/commented.txt"; then ok "comment and blank lines in the table are ignored ($nrows rows)"
else bad "comments rejected: $(tail -3 "$tmp/c")"; fi

echo "$nok ok, $nfail failed"
[ "$nfail" -eq 0 ]
