#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_scanout32/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_scanout32 probe (stage 2 of
# docs/notes/rpi5-display.md section 3: our XRGB8888 plane placed after the
# bootloader's 32-word display list, channel 0's list pointer swapped at
# vblank between two buffers, the firmware's list restored byte-equal) and
# names the result and the answer to section 4 Q4. Same pattern as
# tools/probes/rpi5_display/verdicts.sh: redactor, 'incomplete' handling,
# missing lines named.
#
# A complete log holds these lines, in this order (other lines are ignored).
# The probe's C code prints exactly these formats; the note leaves the
# details open and this header fixes them:
#
#   u3 snapshot: list-ptr=0x<hex> words=<n> hash=0x<hex>
#       the firmware's state before anything is changed: channel 0's list
#       pointer and a checksum/hash of the bootloader's 32-word list
#   u3 dlist: placed word <n> len <n>
#       where our list starts in the list memory and its length in words;
#       <n> of 'word' must be >= 32 (after the firmware's list), len >= 1
#   u3 plane: addr=0x<hex> fmt=XRGB8888 pitch=<n> w=<n> h=<n>
#       fields in any order; fmt must be XRGB8888
#   u3 swap <k>: buf A|B list-ptr=0x<hex> frcnt <before> -> <after>
#       one per swap, k = 1, 2, 3 ... in order, at least 4 swaps
#       (MIN_SWAPS), buf alternating A, B, A, B ...; frcnt is the 6-bit
#       STATUS.FRCNT, decimal, sampled before the pointer write and after
#       the swap settled; it must advance (delta (after-before)&63 of 1..63,
#       so the 6-bit wrap counts; 0 is a stall; scanout_alive.h)
#   u3 named: yes|no <free text>
#       whether channel 0's list pointer read back as our list after the
#       first swap
#   u3 restore: PASS|FAIL <free text>
#       the firmware's list words and pointer byte-equal to the snapshot.
#       A PASS carrying hash=0x<hex> must equal the snapshot's hash
#   rpi5_scanout32: PASS (...)|FAIL (reason)|feeding stopped, reset expected
#
# 'rpi5_scanout32: feeding stopped, reset expected' after a PASS/FAIL line
# is printed too and is not required. A probe prints every line even when
# a step failed: a skipped line makes the log incomplete.
#
# Output: the seen lines, 'missing:' naming absent lines (after the last
# seen one: the image stopped there; between two seen ones: a gap), then
#   verdicts: PASS (...)            all of: placed word >= 32, our list
#                                   named, every swap's FRCNT advanced, the
#                                   buffers alternated, restore PASS, the
#                                   probe's own line not FAIL
#   verdicts: FAIL (reasons)        complete log, one of those not met
#   verdicts: incomplete (...)      a line missing (fewer than MIN_SWAPS
#                                   swap lines count as missing 'u3 swap')
#   q4: yes|no|unanswered (...)     can channel 0's list pointer be swapped
#                                   at vblank under a running scanout:
#                                   from the swap lines, 'yes' needs the
#                                   swaps, FRCNT and naming to hold (the
#                                   restore is judged separately), 'no'
#                                   names the first swap that failed,
#                                   'unanswered' for an incomplete log
# Everything printed goes through the redactor first (IPv4/IPv6, MAC,
# host:port, /dev/tty*, /dev/serial*, ttyS/USB/ACM/AMA names); 0x values
# are not redacted.
#
# Exit status:
#   0  PASS
#   1  incomplete
#   2  usage error (argument count, unreadable file)
#   3  complete log, verdict FAIL
#
# Self-test: tools/probes/rpi_5/rpi5_scanout32/verdicts_selftest.sh
set -u

PROG=${0##*/}
MIN_SWAPS=4

redact_line() {
    # The classes of the display session, over tools/board.sh's: an IPv4
    # or IPv6 address, a MAC address, a host:port pair, a UART device.
    # Over-redacting a look-alike is harmless; printing the value is not.
    # IPv6/MAC runs before host:port so a MAC colon chain is not broken by
    # a host:port match inside it; IPv4 runs last so an address with a
    # port goes whole into the host:port marker.
    sed -E \
        -e 's@(^|[^[:alnum:]_-])tty(S|USB|ACM|AMA)[0-9]+@\1<uart>@g' \
        -e 's@/dev/(tty|serial)[^[:space:]]*@/dev/<uart-path>@g' \
        -e 's@(^|[^[:alnum:]:])((([[:xdigit:]]{1,4}:){2,7}[[:xdigit:]]{1,4})|([[:xdigit:]]{1,4}::[[:xdigit:]:]*))([^[:xdigit:]]|$)@\1<ipv6/mac>\6@g' \
        -e 's@(^|[^[:alnum:]:])((([[:xdigit:]]{1,4}:){2,7}[[:xdigit:]]{1,4})|([[:xdigit:]]{1,4}::[[:xdigit:]:]*))([^[:xdigit:]]|$)@\1<ipv6/mac>\6@g' \
        -e 's@(^|[^[:alnum:]-])(([[:xdigit:]]{2}-){5}[[:xdigit:]]{2})([^[:alnum:]-]|$)@\1<mac>\4@g' \
        -e 's@(^|[^[:alnum:]._-])([[:alnum:]][[:alnum:]._-]*:[0-9]{2,5})([^[:digit:]]|$)@\1<host-port>\3@g' \
        -e 's@(^|[^[:alnum:]._-])([[:alnum:]][[:alnum:]._-]*:[0-9]{2,5})([^[:digit:]]|$)@\1<host-port>\3@g' \
        -e 's@(^|[^[:digit:]])([[:digit:]]{1,3}\.[[:digit:]]{1,3}\.[[:digit:]]{1,3}\.[[:digit:]]{1,3})([^[:alnum:].]|$)@\1<ipv4>\3@g' \
        -e 's@(^|[^[:digit:]])([[:digit:]]{1,3}\.[[:digit:]]{1,3}\.[[:digit:]]{1,3}\.[[:digit:]]{1,3})([^[:alnum:].]|$)@\1<ipv4>\3@g'
}

say() { redact_line <<< "$1"; }

[ $# -eq 1 ] || { echo "usage: $PROG <console.log>" >&2; exit 2; }
LOG=$1
if [ ! -r "$LOG" ]; then
    say "$PROG: cannot read the console log: $LOG" >&2
    exit 2
fi

TMP=$(mktemp "${TMPDIR:-/tmp}/verdicts.XXXXXX") || exit 2
trap 'rm -f "$TMP"' EXIT
tr -d '\r' < "$LOG" > "$TMP"

declare -A SEEN
KEYS=(
    "u3 snapshot"
    "u3 dlist"
    "u3 plane"
    "u3 swap"
    "u3 named"
    "u3 restore"
    "rpi5_scanout32 terminal"
)
SW=()
while IFS=$'\t' read -r idx line; do
    if [ "$idx" = s ]; then SW+=("$line"); else SEEN[$idx]=$line; fi
done < <(awk '
    !seen[1] && /^u3 snapshot: list-ptr=0x[[:xdigit:]]+ words=[0-9]+ hash=0x[[:xdigit:]]+/ { seen[1] = 1; print 1 "\t" $0 }
    !seen[2] && /^u3 dlist: placed word [0-9]+ len [0-9]+/ { seen[2] = 1; print 2 "\t" $0 }
    !seen[3] && /^u3 plane: / { seen[3] = 1; print 3 "\t" $0 }
    /^u3 swap [0-9]+: buf [AB] list-ptr=0x[[:xdigit:]]+ frcnt [0-9]+ -> [0-9]+/ { print "s\t" $0 }
    !seen[5] && /^u3 named: (yes|no)( |[ \t]*$)/ { seen[5] = 1; print 5 "\t" $0 }
    !seen[6] && /^u3 restore: (PASS|FAIL)( |[ \t]*$)/ { seen[6] = 1; print 6 "\t" $0 }
    !seen[7] && /^rpi5_scanout32: (PASS|FAIL|feeding stopped, reset expected)/ { seen[7] = 1; print 7 "\t" $0 }
    !seen[8] && /^rpi5_scanout32: feeding stopped, reset expected/ { seen[8] = 1; print 8 "\t" $0 }
' "$TMP")
NSW=${#SW[@]}
[ "$NSW" -ge "$MIN_SWAPS" ] && SEEN[4]=${SW[0]}

nseen=0
LASTKEY=0
for i in 1 2 3 4 5 6 7; do
    [ -n "${SEEN[$i]:-}" ] && { nseen=$((nseen + 1)); LASTKEY=$i; }
done
# a few swap lines (fewer than MIN_SWAPS) still show where the image stopped
if [ "$NSW" -gt 0 ] && [ "$LASTKEY" -lt 4 ]; then LASTKEY=4; fi
LASTLINE=${SEEN[$LASTKEY]:-}
if [ "$LASTKEY" -eq 4 ] && [ "$NSW" -gt 0 ]; then LASTLINE=${SW[$((NSW - 1))]}; fi

say "$PROG: $nseen of 7 expected verdict lines in $LOG ($NSW swap line(s))"
for i in 1 2 3; do [ -n "${SEEN[$i]:-}" ] && say "  [$i] ${SEEN[$i]}"; done
for ((j = 0; j < NSW; j++)); do say "  [4] ${SW[$j]}"; done
for i in 5 6 7; do [ -n "${SEEN[$i]:-}" ] && say "  [$i] ${SEEN[$i]}"; done
if [ -n "${SEEN[8]:-}" ] && [ "${SEEN[8]}" != "${SEEN[7]:-}" ]; then say "  [8] ${SEEN[8]}"; fi

MISS="" GAP="" TAIL=""
for i in 1 2 3 4 5 6 7; do
    [ -n "${SEEN[$i]:-}" ] && continue
    name=${KEYS[$((i - 1))]}
    [ "$i" -eq 4 ] && name="u3 swap (need $MIN_SWAPS, saw $NSW)"
    MISS="$MISS $name"
    if [ "$i" -le "$LASTKEY" ]; then GAP="$GAP $name"; else TAIL="$TAIL $name"; fi
done
if [ -n "$MISS" ]; then
    say "  missing:${MISS}"
    if [ -n "$TAIL" ]; then
        if [ "$LASTKEY" -gt 0 ]; then
            say "  after the last seen line [${LASTKEY}] ${LASTLINE}: the image stopped there"
        else
            say "  no expected verdict line was seen at all"
        fi
    fi
    [ -n "$GAP" ] && say "  gap: missing between seen lines:${GAP}"
else
    say "  missing: none"
fi

if [ "$nseen" -lt 7 ]; then
    say "verdicts: incomplete (the run did not print all expected verdict lines)"
    say "q4: unanswered (incomplete log)"
    exit 1
fi

# ---- complete log: judge ---------------------------------------------------
swap_fail=""
other=()
prev_buf="" expect=1
for ((j = 0; j < NSW; j++)); do
    l=${SW[$j]}
    k=$(sed -E 's/^u3 swap ([0-9]+):.*/\1/' <<< "$l")
    buf=$(sed -E 's/^u3 swap [0-9]+: buf ([AB]).*/\1/' <<< "$l")
    before=$(sed -E 's/.* frcnt ([0-9]+) -> ([0-9]+).*/\1/' <<< "$l")
    after=$(sed -E 's/.* frcnt ([0-9]+) -> ([0-9]+).*/\2/' <<< "$l")
    if [ "$k" -ne "$expect" ]; then
        [ -z "$swap_fail" ] && swap_fail="swap numbering broke at $k (expected $expect)"
        other+=("swap numbering broke at $k (expected $expect)")
    fi
    expect=$((k + 1))
    if [ "$before" -gt 63 ] || [ "$after" -gt 63 ]; then
        [ -z "$swap_fail" ] && swap_fail="swap $k: frcnt out of the 6-bit range ($before -> $after)"
        other+=("swap $k: frcnt out of 6-bit range")
    else
        d=$(( (after - before) & 63 ))
        if [ "$d" -eq 0 ]; then
            [ -z "$swap_fail" ] && swap_fail="FRCNT stalled at swap $k ($before -> $after)"
            other+=("FRCNT stalled at swap $k")
        fi
    fi
    if [ -n "$prev_buf" ] && [ "$buf" = "$prev_buf" ]; then
        [ -z "$swap_fail" ] && swap_fail="buffers did not alternate at swap $k (buf $buf twice)"
        other+=("buffers did not alternate at swap $k")
    fi
    prev_buf=$buf
done

named=$(grep -oE '(yes|no)' <<< "${SEEN[5]}" | head -n 1)
if [ "$named" != "yes" ]; then
    [ -z "$swap_fail" ] && swap_fail="channel 0 did not name our list after the first swap"
    other+=("our list was not named")
fi

word=$(sed -E 's/^u3 dlist: placed word ([0-9]+) len ([0-9]+).*/\1/' <<< "${SEEN[2]}")
len=$(sed -E 's/^u3 dlist: placed word ([0-9]+) len ([0-9]+).*/\2/' <<< "${SEEN[2]}")
[ "$word" -ge 32 ] || other+=("placed word $word < 32, inside the firmware's list")
[ "$len" -ge 1 ] || other+=("placed list length 0")

grep -qE '\bfmt=XRGB8888( |$)' <<< "${SEEN[3]}" || other+=("plane fmt is not XRGB8888")

rst=$(grep -oE 'PASS|FAIL' <<< "${SEEN[6]}" | head -n 1)
[ "$rst" = "PASS" ] || other+=("restore FAIL")
if [ "$rst" = "PASS" ]; then
    rh=$(grep -oE 'hash=0x[[:xdigit:]]+' <<< "${SEEN[6]}" | head -n 1)
    sh=$(grep -oE 'hash=0x[[:xdigit:]]+' <<< "${SEEN[1]}" | head -n 1)
    if [ -n "$rh" ] && [ "${rh,,}" != "${sh,,}" ]; then
        other+=("restore PASS carries $rh but the snapshot has $sh")
    fi
fi

grep -qE '^rpi5_scanout32: FAIL' <<< "${SEEN[7]}" && other+=("the probe's own line is FAIL")

if [ -n "$swap_fail" ]; then
    say "q4: no ($swap_fail)"
else
    say "q4: yes (channel 0's list pointer swapped $NSW times under the running scanout, named after the first swap, FRCNT advanced through every swap)"
fi

if [ ${#other[@]} -eq 0 ]; then
    say "verdicts: PASS ($NSW swaps alternating, our list named at word $word, FRCNT advanced, firmware list restored byte-equal)"
    exit 0
fi
r=""
for x in "${other[@]}"; do r="$r${r:+; }$x"; done
say "verdicts: FAIL ($r)"
exit 3
