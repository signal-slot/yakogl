#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_clear/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_clear image (src/main.c) and prints, in
# this order:
#
#   verdicts: m1: <ok|FAIL|missing> (mailbox, power_on, identity V3D 7.1)
#   verdicts: m2: <...> (mmu_init)
#   verdicts: m3: <...> (bin, render, caches_clean, each 0)
#   verdicts: m4: <...> (0 pixels wrong, 0 guard row words changed)
#   verdicts: end: <the image's `v3d_clear: PASS|FAIL` line, or missing>
#   verdicts: PASS | FAIL (<reason>)
#
# PASS needs the end line to say PASS and every step line above to be there
# and clean: an end line that says PASS beside a wrong read-back is a FAIL.
# Lines may carry a timestamp or log prefix and CR line ends.
#
# Exit: 0 done (PASS), 1 no end line (truncated log), 2 usage, 3 FAIL.
set -u

if [ $# -ne 1 ] || [ ! -r "$1" ]; then
    echo "usage: verdicts.sh <console.log>" >&2
    exit 2
fi

tmp=$(mktemp "${TMPDIR:-/tmp}/rpi5_clear_v.XXXXXX") || exit 2
trap 'rm -f "$tmp"' EXIT INT TERM
tr -d '\r' <"$1" >"$tmp"

# last line containing a fixed string
last() { grep -F -- "$1" "$tmp" | tail -n 1; }

bad=""
add() { bad="${bad:+$bad; }$1"; }

# zero_step <prefix>: ok when the line ends in "<prefix> 0"
zero_step() {
    l=$(last "$1")
    case "$l" in
    "") echo "missing" ;;
    *"$1 0") echo "ok" ;;
    *) echo "FAIL" ;;
    esac
}

mbox=$(zero_step "M1 mailbox:")
pwr=$(last "M1 power_on:")
case "$pwr" in "") pwr=missing ;; *"M1 power_on: 0,"*) pwr=ok ;; *) pwr=FAIL ;; esac
idl=$(last "M1 identity: V3D ")
case "$idl" in "") idl=missing ;; *"V3D 7.1,"*": 0") idl=ok ;; *) idl=FAIL ;; esac
mmu=$(zero_step "M2 mmu_init:")
bin=$(zero_step "M3 bin:")
rnd=$(zero_step "M3 render:")
cln=$(zero_step "M3 caches_clean:")
m4l=$(last "M4: ")
case "$m4l" in
"") m4=missing ;;
*"M4: 0 of "*" target pixels wrong, guard row 0 of "*) m4=ok ;;
*) m4=FAIL ;;
esac

group() {
    s=ok
    for x in "$@"; do
        [ "$x" = FAIL ] && s=FAIL
        [ "$x" = missing ] && [ "$s" = ok ] && s=missing
    done
    echo "$s"
}
m1=$(group "$mbox" "$pwr" "$idl")
m2=$(group "$mmu")
m3=$(group "$bin" "$rnd" "$cln")

echo "verdicts: m1: $m1 (mailbox $mbox, power_on $pwr, identity $idl)"
echo "verdicts: m2: $m2 (mmu_init $mmu)"
echo "verdicts: m3: $m3 (bin $bin, render $rnd, caches_clean $cln)"
echo "verdicts: m4: $m4 (${m4l:-no M4 line})"

end=$(grep -E 'v3d_clear: (PASS|FAIL) \(' "$tmp" | tail -n 1)
if [ -z "$end" ]; then
    echo "verdicts: end: missing"
    echo "verdicts: FAIL (no end line)"
    exit 1
fi
echo "verdicts: end: v3d_clear:${end#*v3d_clear:}"

case "$end" in
*"v3d_clear: FAIL"*) add "image says FAIL" ;;
esac
for p in "m1 $m1" "m2 $m2" "m3 $m3" "m4 $m4"; do
    case "${p#* }" in
    ok) ;;
    *) add "${p% *} ${p#* }" ;;
    esac
done

if [ -n "$bad" ]; then
    echo "verdicts: FAIL ($bad)"
    exit 3
fi
echo "verdicts: PASS (M1 to M4 clean)"
exit 0
