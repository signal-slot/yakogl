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

for n in clean faulty zephyr; do
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

# A slice of the Verdin iMX95 is a Zephyr U-Boot slice whichever way its
# meta.txt spells the board: verdin-imx95, verdin_imx95 (the committed slices)
# or verdin-imx95-zephyr. It gets the same README, and never the panthor or
# own-Linux wording of the T6's route.
for spell in verdin-imx95 verdin_imx95; do
    rm -rf "$work/v"; mkdir "$work/v"; cp -r "$DATA/zephyr" "$work/v/zephyr"
    sed -i "s/^board=.*/board=$spell/" "$work/v/zephyr/meta.txt"
    got=$(python3 -I "$TOOL" "$work/v/zephyr" 2>&1)
    if [ "$got" = "$(cat "$DATA/zephyr.expected")" ] &&
       grep -q 'ext4load. or .fatload.' <<<"$got" && grep -q 'dcache off; icache flush; go 0xd0000000' <<<"$got" &&
       grep -q 'UART_3 console' <<<"$got" && ! grep -qi 'panthor\|own Linux\|NanoPC\|Mali-G610\|mali-deqp-run' <<<"$got"
    then ok "board=$spell: the README names the Zephyr U-Boot route"
    else bad "board=$spell: the README does not name the Zephyr U-Boot route"; fi
done

# The Zephyr Verdin iMX95 slice (board=verdin-imx95-zephyr, no kmsg/, rows.tsv)
# never claims Linux, panthor, kmsg, ssh or a boot_id, and shows the rows.
z=$DATA/zephyr.expected
if ! grep -qiE 'linux|panthor|kmsg|ssh|boot_id|armbian|mali-deqp-run' "$z" &&
   grep -q 'Mali-G310 under Zephyr, started from U-Boot' "$z" &&
   grep -q '^## Rows' "$z"; then ok "zephyr: README claims no Linux, panthor or kmsg"; else bad "zephyr: README claims Linux, panthor or kmsg"; fi
rm -rf "$work/z"; cp -r "$DATA/zephyr" "$work/z"
sed -i 's/^1\tT6S9R01\t12\t360/1\tT6S9R01\t400\t360/; s/yes$/no/' "$work/z/rows.tsv"
got=$(python3 -I "$TOOL" "$work/z" 2>&1)
if grep -q '1 reached their lifetime' <<<"$got" && grep -q '^| no | 2 |' <<<"$got"; then ok "zephyr: rows over lifetime and bbm counted"; else bad "zephyr: rows.tsv not summarised"; fi
printf 'row\tlabel\tseconds\n1\tx\t3\n' > "$work/z/rows.tsv"
python3 -I "$TOOL" "$work/z" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "zephyr: rows.tsv without lifetime_s or bbm is refused" || bad "zephyr: bad rows.tsv exit $rc"
rm "$work/z/rows.tsv"
python3 -I "$TOOL" "$work/z" >/dev/null 2>&1; rc=$?
[ "$rc" = 0 ] && ok "zephyr: rows.tsv is optional" || bad "zephyr: no rows.tsv exit $rc"

# A missing file is refused too.
rm -rf "$work/r"; cp -r "$DATA/clean" "$work/r"; rm "$work/r/chunks.tsv"
python3 -I "$TOOL" "$work/r" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "refuses a slice without chunks.tsv" || bad "missing chunks.tsv: exit $rc"

if [ "$fail" = 0 ]; then echo "mali-slice-readme-selftest: ok"; exit 0; fi
echo "mali-slice-readme-selftest: $fail failed"
exit 1
