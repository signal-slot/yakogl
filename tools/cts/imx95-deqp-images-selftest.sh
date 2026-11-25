#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of `imx95-deqp-images.sh --dry-run` over mali-gles2-tree.txt: needs
# neither the Zephyr nor the dEQP tree, builds nothing.
#   (a) the dry run names the table's labels, once each, in table order
#   (b) the wrapper passes the board verdin_imx95/mimx9596/a55 (a stub
#       builder records its arguments)
#   (c) a LABEL argument selects only that row
#   (d) a malformed table is refused with a non-zero exit; an unknown label is
#       refused only if the scripts document it (they do not: SKIP)
# Usage: sh tools/cts/imx95-deqp-images-selftest.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/imx95-images-selftest.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
table=$here/mali-gles2-tree.txt
pass=0
fail=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }
check() { if [ "$1" = 0 ]; then ok "$2"; else bad "$2"; fi; }
labels() { awk '{ print $1 }'; }

want=$(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$table")
n=$(echo "$want" | wc -l)
[ "$n" -eq 76 ]
check $? "the table has 76 image labels (found $n)"

# (a)
out=$(bash "$here/imx95-deqp-images.sh" --dry-run "$table" "$tmp/img" 2>&1)
check $? "(a) dry run exits 0"
got=$(echo "$out" | labels)
[ "$got" = "$want" ]
check $? "(a) dry run names the $n labels once each, in table order"
[ ! -e "$tmp/img" ]
check $? "(a) dry run builds nothing"

# (b) the wrapper copied next to a stub builder that records its arguments
mkdir "$tmp/stub"
cp "$here/imx95-deqp-images.sh" "$tmp/stub/"
printf '#!/bin/sh\necho "$@" > "%s/args"\n' "$tmp" > "$tmp/stub/deqp-tree-images.sh"
chmod +x "$tmp/stub/deqp-tree-images.sh"
bash "$tmp/stub/imx95-deqp-images.sh" --dry-run "$table" "$tmp/img" > /dev/null 2>&1
[ "$(cat "$tmp/args" 2>/dev/null)" = "verdin_imx95/mimx9596/a55 --dry-run $table $tmp/img" ]
check $? "(b) board verdin_imx95/mimx9596/a55 is the builder's first argument"

# (c)
l1=$(echo "$want" | sed -n 1p)
l2=$(echo "$want" | sed -n 40p)
out=$(bash "$here/imx95-deqp-images.sh" --dry-run "$table" "$tmp/img" "$l2" 2>&1)
[ "$(echo "$out" | labels)" = "$l2" ]
check $? "(c) one LABEL ($l2) selects only that row"
out=$(bash "$here/imx95-deqp-images.sh" --dry-run "$table" "$tmp/img" "$l2" "$l1" 2>&1)
[ "$(echo "$out" | labels)" = "$(printf '%s\n%s' "$l1" "$l2")" ]
check $? "(c) two LABELs select those two rows, in table order"

# (d)
printf '# image A1: x\nCONFIG_DEQP_CASE=ok\nstray filter\n' > "$tmp/bad.txt"
bash "$here/imx95-deqp-images.sh" --dry-run "$tmp/bad.txt" "$tmp/img" > /dev/null 2>&1
rc=$?
[ "$rc" -ne 0 ]
check $? "(d) a filter line without a label is refused (rc=$rc)"
bash "$here/imx95-deqp-images.sh" --dry-run "$tmp/none.txt" "$tmp/img" > /dev/null 2>&1
rc=$?
[ "$rc" -ne 0 ]
check $? "(d) a missing table is refused (rc=$rc)"
echo "SKIP (d) unknown label refused: the headers do not document it (a dry run with one prints nothing and exits 0)"

# the check must notice a lost label
grep -v "^# image $l1:" "$table" > "$tmp/short.txt"
out=$(bash "$here/imx95-deqp-images.sh" --dry-run "$tmp/short.txt" "$tmp/img" 2>&1)
[ "$(echo "$out" | labels)" != "$want" ]
check $? "(a) a table without $l1 gives a different label list"

echo "$pass ok, $fail failed"
[ "$fail" = 0 ]
