#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pi4-abort-causes-selftest.sh
#
# Host-only check of tools/cts/pi4-abort-causes.py. It checks:
#   - the 14 consoles of tools/cts/log/2026-11-pi4-gles3-b1b5b845-aborts/
#     (read in place, already redacted) against
#     testdata/pi4-abort-causes/b1b5b845-aborts.expected.txt: the 11 refusing
#     cases of that README with their class (S03a tile-memory; T05ca T05cb
#     T05cc T17a texel-format; T14a gpu-memory; TF1a TF1b TF2a TF2b TF2c
#     job-builder) and DONE for T14b, T17b, TF1c
#   - the same, one file at a time in a different order (the output follows
#     the arguments)
#   - --refuse-list against b1b5b845-aborts.refuse.txt, every line in the
#     two-field 'LABEL CASE' shape tools/cts/pi4-gles3-resplit.py's
#     --refuse-file reads
#   - the hand-written consoles of testdata/pi4-abort-causes/edge/ against
#     edge.expected.txt: a cause logged once by an earlier case (E1), the
#     record's cause list only, with time stamps (E2), "no empty job" without
#     a memory line (E3, unknown), a refusal of no known class (E4, unknown),
#     no case begun (E5), a console that stops without a ResourceError (E6)
#   - bad input: a missing file and no arguments exit 2
set -u
# The reverse-order check sorts with sort(1) and a glob: byte order, whatever
# the runner's locale.
export LC_ALL=C

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/pi4-abort-causes.py
FIX=$HERE/testdata/pi4-abort-causes
SESSION=$HERE/log/2026-11-pi4-gles3-b1b5b845-aborts
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

WORK=$(mktemp -d "${TMPDIR:-.}/pi4-abort-causes.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT

# Paths print by label only, so the output does not depend on where it runs.
if python3 "$TOOL" "$SESSION" > "$WORK/session.txt" 2> "$WORK/err" &&
   diff -u "$FIX/b1b5b845-aborts.expected.txt" "$WORK/session.txt"; then
    ok "b1b5b845 aborts session: 11 refusing cases and 3 DONE"
else
    bad "b1b5b845 aborts session"; cat "$WORK/err"
fi

n_abort=$(awk -F'\t' 'NF == 4' "$WORK/session.txt" | wc -l)
n_done=$(awk -F'\t' '$2 == "DONE"' "$WORK/session.txt" | wc -l)
if [ "$n_abort" -eq 11 ] && [ "$n_done" -eq 3 ] && ! grep -q 'unknown' "$WORK/session.txt"; then
    ok "11 aborts, 3 DONE, no unknown class"
else
    bad "counts: $n_abort aborts, $n_done DONE"
fi

for c in "S03a tile-memory" "T05ca texel-format" "T05cb texel-format" "T05cc texel-format" \
         "T14a gpu-memory" "T17a texel-format" "TF1a job-builder" "TF1b job-builder" \
         "TF2a job-builder" "TF2b job-builder" "TF2c job-builder"; do
    set -- $c
    if awk -F'\t' -v l="$1" -v k="$2" '$1 == l && $3 == k { f = 1 } END { exit !f }' "$WORK/session.txt"; then
        ok "$1 is $2"
    else
        bad "$1 is $2"
    fi
done

# Reverse order, one file per argument.
files=()
for f in $(ls "$SESSION"/*.console.txt | sort -r); do files+=("$f"); done
if python3 "$TOOL" "${files[@]}" > "$WORK/rev.txt" &&
   sort -r "$FIX/b1b5b845-aborts.expected.txt" | cmp -s - "$WORK/rev.txt"; then
    ok "output follows the argument order"
else
    bad "argument order"
fi

if python3 "$TOOL" --refuse-list "$SESSION" > "$WORK/refuse.txt" &&
   diff -u "$FIX/b1b5b845-aborts.refuse.txt" "$WORK/refuse.txt" &&
   awk 'NF != 2 { exit 1 }' "$WORK/refuse.txt"; then
    ok "--refuse-list: 'LABEL CASE' for the 11 aborts"
else
    bad "--refuse-list"
fi

if python3 "$TOOL" "$FIX/edge" > "$WORK/edge.txt" 2> "$WORK/err" &&
   diff -u "$FIX/edge.expected.txt" "$WORK/edge.txt"; then
    ok "edge consoles E1-E6"
else
    bad "edge consoles"; cat "$WORK/err"
fi

python3 "$TOOL" "$WORK/no-such.console.txt" > /dev/null 2>&1
[ $? -eq 2 ] && ok "a missing file exits 2" || bad "a missing file exits 2"
python3 "$TOOL" > /dev/null 2>&1
[ $? -eq 2 ] && ok "no arguments exit 2" || bad "no arguments exit 2"

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
