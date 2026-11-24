#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_triangle/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_triangle or rpi5_triangle_ours image
# (src/main.c, the same sources) and prints, in this order:
#
#   verdicts: build: <the shaders named in the build line: mesa|ours|missing>
#   verdicts: m1: <ok|FAIL|missing> (mailbox, power_on, identity V3D 7.1)
#   verdicts: m2: <...> (mmu_init)
#   verdicts: m5: <...> (bin, render, caches_clean, each 0)
#   verdicts: pixels: <ok|FAIL|missing> (five `M5 pixel` lines, read-back
#                       rc 0, 0 wrong inside/outside/edge, 0 guard bytes)
#   verdicts: end: <the image's `v3d_triangle: PASS|FAIL` line, or missing>
#   verdicts: PASS | FAIL (<reason>)
#
# PASS needs the end line to say PASS, every step line above to be there and
# clean, and the shaders name of the build line to be mesa or ours and the
# same as the end line's: an end line that says PASS beside a non-zero M5 rc
# or a missing identity line is a FAIL. Lines may carry a timestamp or log
# prefix and CR line ends.
#
# Exit: 0 done (PASS), 1 no end line (truncated log), 2 usage, 3 FAIL.
set -u

if [ $# -ne 1 ] || [ ! -r "$1" ]; then
    echo "usage: verdicts.sh <console.log>" >&2
    exit 2
fi

tmp=$(mktemp "${TMPDIR:-/tmp}/rpi5_triangle_v.XXXXXX") || exit 2
trap 'rm -f "$tmp"' EXIT INT TERM
tr -d '\r' <"$1" >"$tmp"

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
# rc_step <prefix>: ok when the line has "<prefix> 0," (rc first, then data)
rc_step() {
    l=$(last "$1")
    case "$l" in
    "") echo "missing" ;;
    *"$1 0,"*) echo "ok" ;;
    *) echo "FAIL" ;;
    esac
}

bl=$(last "v3d_triangle: build ")
case "$bl" in
"") bsh=missing ;;
*", shaders mesa "*) bsh=mesa ;;
*", shaders ours "*) bsh=ours ;;
*) bsh=FAIL ;;
esac

mbox=$(zero_step "M1 mailbox:")
pwr=$(rc_step "M1 power_on:")
idl=$(last "M1 identity: V3D ")
case "$idl" in "") idl=missing ;; *"V3D 7.1,"*": 0") idl=ok ;; *) idl=FAIL ;; esac
mmu=$(zero_step "M2 mmu_init:")
bin=$(rc_step "M5 bin:")
rnd=$(rc_step "M5 render:")
cln=$(zero_step "M5 caches_clean:")

npx=$(grep -c 'M5 pixel .* ([0-9]*,[0-9]*): [0-9a-f][0-9a-f] ' "$tmp")
rb=$(last "M5 read-back: ")
case "$rb" in
"") px=missing ;;
*"M5 read-back: 0, "*" inside (0 wrong, max diff "*"), "*" outside (0 wrong), "*" edge (0 wrong), guard rows 0 of "*)
    if [ "$npx" -ge 5 ]; then px=ok; else px=missing; fi
    ;;
*) px=FAIL ;;
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
m5=$(group "$bin" "$rnd" "$cln")

echo "verdicts: build: $bsh"
echo "verdicts: m1: $m1 (mailbox $mbox, power_on $pwr, identity $idl)"
echo "verdicts: m2: $m2 (mmu_init $mmu)"
echo "verdicts: m5: $m5 (bin $bin, render $rnd, caches_clean $cln)"
echo "verdicts: pixels: $px (${rb:-no read-back line}; $npx pixel lines)"

end=$(grep -E 'v3d_triangle: (PASS|FAIL) \(' "$tmp" | tail -n 1)
if [ -z "$end" ]; then
    echo "verdicts: end: missing"
    echo "verdicts: FAIL (no end line)"
    exit 1
fi
echo "verdicts: end: v3d_triangle:${end#*v3d_triangle:}"

case "$end" in
*"v3d_triangle: FAIL"*) add "image says FAIL" ;;
esac
case "$end" in
*"; shaders mesa)"*) esh=mesa ;;
*"; shaders ours)"*) esh=ours ;;
*) esh=FAIL ;;
esac
if [ "$esh" = FAIL ]; then
    add "end line shaders name"
elif [ "$bsh" != "$esh" ]; then
    add "build shaders $bsh, end line $esh"
fi
for p in "m1 $m1" "m2 $m2" "m5 $m5" "pixels $px"; do
    case "${p#* }" in
    ok) ;;
    *) add "${p% *} ${p#* }" ;;
    esac
done

if [ -n "$bad" ]; then
    echo "verdicts: FAIL ($bad)"
    exit 3
fi
echo "verdicts: PASS (M1 to M5 clean, shaders $esh)"
exit 0
