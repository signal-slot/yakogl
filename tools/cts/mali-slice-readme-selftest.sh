#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mali-slice-readme-selftest.sh
#
# Host-only check of tools/cts/mali-slice-readme.py on the fixture slices in
# tools/cts/testdata/mali-slice-readme/: clean/ (two done chunks) and faulty/
# (a crash, a hang with a panthor reset, Lost rows). The printed README must
# equal NAME.expected; --write then --check must agree; --check must exit 1 on
# a stale or missing README; a slice with an IP address or user@host must be
# refused. Exits 0 only if every comparison matches.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mali-slice-readme.py
DATA=$HERE/testdata/mali-slice-readme
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

work=$(mktemp -d "${TMPDIR:-/tmp}/mali-slice-readme.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

for n in clean faulty; do
    got=$(python3 -I "$TOOL" "$DATA/$n" 2>&1)
    if [ "$got" = "$(cat "$DATA/$n.expected")" ]; then ok "$n: output"; else bad "$n: output differs from $n.expected"; fi

    cp -r "$DATA/$n" "$work/$n"
    python3 -I "$TOOL" --check "$work/$n" >/dev/null 2>&1; rc=$?
    [ "$rc" = 1 ] && ok "$n: --check exits 1 with no README" || bad "$n: --check with no README exited $rc"
    python3 -I "$TOOL" --write "$work/$n" || bad "$n: --write failed"
    if cmp -s "$work/$n/README.md" "$DATA/$n.expected"; then ok "$n: --write bytes"; else bad "$n: --write bytes differ"; fi
    python3 -I "$TOOL" --check "$work/$n" >/dev/null 2>&1; rc=$?
    [ "$rc" = 0 ] && ok "$n: --check exits 0 on a fresh README" || bad "$n: --check on a fresh README exited $rc"
    echo "stale" >> "$work/$n/README.md"
    python3 -I "$TOOL" --check "$work/$n" >/dev/null 2>&1; rc=$?
    [ "$rc" = 1 ] && ok "$n: --check exits 1 on a stale README" || bad "$n: --check on a stale README exited $rc"
done

# The faulty slice's README says what went wrong.
for pat in 'crash' 'hang' 'chunk-crash' 'chunk-hang' 'not-run'; do
    grep -q "$pat" "$DATA/faulty.expected" && ok "faulty: names $pat" || bad "faulty: no $pat"
done

# A README changed by a changed result is stale.
cp -r "$DATA/clean" "$work/changed"
python3 -I "$TOOL" --write "$work/changed"
sed -i 's/\tPass\tPass$/\tFail\tmismatch/' "$work/changed/results.tsv"
python3 -I "$TOOL" --check "$work/changed" >/dev/null 2>&1; rc=$?
[ "$rc" = 1 ] && ok "changed results: --check exits 1" || bad "changed results: --check exited $rc"

# Refusals: an address or user@host anywhere in the slice.
ip="10.1.2.3"
for what in "ip:results.tsv:dEQP-GLES2.x.y\tFail\tconnect to $ip" \
            "userhost:meta.txt:kernel=root@board" \
            "ip-kmsg:kmsg/info.txt:link up at $ip"; do
    IFS=: read -r tag file line <<<"$what"
    rm -rf "$work/r"; cp -r "$DATA/clean" "$work/r"
    printf "$line\n" >> "$work/r/$file"
    out=$(python3 -I "$TOOL" "$work/r" 2>&1); rc=$?
    if [ "$rc" = 2 ] && ! grep -q 'Mali-G610' <<<"$out"; then ok "refuses $tag"; else bad "$tag: exit $rc, not refused"; fi
    python3 -I "$TOOL" --write "$work/r" >/dev/null 2>&1
    [ -e "$work/r/README.md" ] && bad "$tag: --write wrote a README" || ok "$tag: --write wrote nothing"
done

# A slice of another board (a board= line in meta.txt, mali-deqp-run.sh on
# MALI_BOARD=verdin-imx95) names that board and the command that made it;
# everything else is the clean slice's README.
rm -rf "$work/v"; mkdir -p "$work/v"; cp -r "$DATA/clean" "$work/v/clean"
sed -i 's/^rows=.*/&\nboard=verdin-imx95/' "$work/v/clean/meta.txt"
got=$(python3 -I "$TOOL" "$work/v/clean" 2>&1)
if grep -q '^# Verdin iMX95 dEQP-GLES2 ' <<<"$got" && grep -q 'of the Verdin iMX95 (Mali-G310 over panthor' <<<"$got" &&
   grep -q '^MALI_BOARD=verdin-imx95 BOARD_HOST=<board> tools/cts/mali-deqp-run.sh' <<<"$got" &&
   ! grep -q 'NanoPC-T6\|Mali-G610\|nanopc-t6' <<<"$got"; then ok "board=verdin-imx95: the README names the Verdin iMX95 only"
else bad "board=verdin-imx95: the README does not name the board"; fi
want=$(sed -e 's/NanoPC-T6/Verdin iMX95/g' -e 's/Mali-G610 over panthor, the board.s own Armbian/Mali-G310 over panthor, the board'"'"'s own Linux/' \
  -e 's/^BOARD_HOST=<board>/MALI_BOARD=verdin-imx95 BOARD_HOST=<board>/' -e 's/target:nanopc-t6/target:verdin-imx95/' "$DATA/clean.expected")
if [ "$got" = "$want" ]; then ok "board=verdin-imx95: no other line differs from clean.expected"; else bad "board=verdin-imx95: other lines differ"; fi

# A missing file is refused too.
rm -rf "$work/r"; cp -r "$DATA/clean" "$work/r"; rm "$work/r/chunks.tsv"
python3 -I "$TOOL" "$work/r" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "refuses a slice without chunks.tsv" || bad "missing chunks.tsv: exit $rc"

if [ "$fail" = 0 ]; then echo "mali-slice-readme-selftest: ok"; exit 0; fi
echo "mali-slice-readme-selftest: $fail failed"
exit 1
