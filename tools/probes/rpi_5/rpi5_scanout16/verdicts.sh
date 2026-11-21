#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_scanout16/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_scanout16 probe (frame 1 of the Pi 5
# display, docs/notes/rpi5-display.md section 3: adopt the RGB565 plane the
# firmware left running, draw 16 bpp fbcon text and a pattern, read back
# twice, hold) and names, on one line, the verdict of the run:
#
#   verdicts: PASS (...)         every condition below holds, exit 0
#   verdicts: FAIL (reasons)     a complete log where one does not, exit 3
#   verdicts: incomplete (...)   an expected line is missing, exit 1
#
# PASS needs all of: the plane adopted ('u3 adopt-readback: PASS'), both
# read-backs PASS, the frame counter advanced, PV_CONTROL.EN = 1 and
# PV_V_CONTROL.VIDEN = 1. The long-hold line does not enter the verdict: it
# answers Q2 of the note (does the firmware redraw into its plane, or change
# its display list, while we hold it) on a separate 'q2: ...' line.
#
# A complete log holds these lines, in this order, once each (other log
# lines are ignored). <n> is decimal, 0x<hex> is hex, a reason is free text:
#
#   [1]  u3 scanout: running|stopped ...     Z0/Z1 snapshot, as rpi5_display
#   [2]  u3 plane: addr=0x<hex> fmt=<f> pitch=<n> w=<n> h=<n>
#                                            fields in any order, as rpi5_display
#   [3]  u3 adopt-readback: PASS|FAIL ...
#   [4]  rpi5_scanout16: text rows <n> changed
#                                            fbcon rows that changed plane bytes
#   [5]  rpi5_scanout16: pattern drawn|not drawn (...)
#   [6]  u3 readback 1: PASS|FAIL ...        after the draw
#   [7]  u3 readback 2: PASS|FAIL ...        after the hold, same bytes expected
#   [8]  u3 frcnt: before <n> after <n>      STATUS.FRCNT of the channel, 0..63
#   [9]  u3 pv: control=0x<hex> en=<0|1> viden=<0|1>
#                                            PV_CONTROL bit 0 (EN) and
#                                            PV_V_CONTROL bit 0 (VIDEN)
#   [10] u3 hold: <ms> ms plane=<kept|redrawn> bytes=<n> list=<kept|changed> words=<n>
#                                            hold length; plane=redrawn when the
#                                            plane bytes differ from what the probe
#                                            wrote, bytes = how many; list=changed
#                                            when the channel's display-list words
#                                            differ from the adopted snapshot,
#                                            words = how many
#   [11] rpi5_scanout16: PASS (...)|FAIL (reason)|feeding stopped, reset expected
#
# A line whose payload does not match its format ([4], [5], [8], [9] and [10]
# are matched in full, [3], [6] and [7] on PASS|FAIL) does not count: it is
# reported missing. The last line accepts a result line or the planned
# watchdog ending. 'rpi5_scanout16: feeding stopped, reset expected'
# following a PASS/FAIL line is extracted and printed too, and is not
# required. A probe prints every line even when an earlier step failed: a
# skipped line makes the log incomplete even when a result line closed the run.
#
# Frame counter: FRCNT is 6 bits (driver/zephyr/lib/board/rpi5_scanout/
# scanout_alive.h) and wraps, so 'advanced' is (after - before) mod 64 != 0;
# 60 -> 3 advanced, 17 -> 17 did not. A value above 63 is not a counter: the
# verdict is FAIL (the sampling gap is tens of ms, never a whole 64 frames).
#
# Q2 line (printed whenever the log is read; never changes the exit status):
#   q2: no (the firmware neither redrew the plane nor changed its list in <ms> ms)
#   q2: yes, redrawn (<bytes> plane bytes changed in <ms> ms)
#   q2: yes, list changed (<words> list words changed in <ms> ms)
#   q2: yes, redrawn and list changed (<bytes> plane bytes, <words> list words in <ms> ms)
#   q2: unclear (<why>)    the counts contradict the kept/redrawn words, or ms is 0
#   q2: unanswered (no hold line)
#
# Missing expected lines are named. Missing lines after the last seen one
# mean the image stopped there; missing lines between two seen ones are a
# gap. An incomplete log gets no PASS or FAIL: the line reads 'incomplete'
# and the exit is 1. If the probe's own terminal line says FAIL while the
# evidence passes, the verdict is FAIL (the probe saw something this script
# cannot); if it says PASS while the evidence fails, the evidence wins.
#
# Everything the script prints goes through a redactor first, like
# tools/board.sh's redact_line: IPv4 and IPv6 addresses, MAC addresses,
# host:port pairs, serial device paths and tty names (ttyS,
# USB, ACM and AMA ones) come out as markers; rules run twice so a second
# value next to an already masked one is caught too. A DRAM address and
# other 0x... values print unredacted.
#
# Exit status:
#   0  complete and PASS
#   1  incomplete: an expected line is missing
#   2  usage error (argument count, an unreadable file)
#   3  complete and FAIL
#
# Self-test: tools/probes/rpi_5/rpi5_scanout16/verdicts_selftest.sh
set -u

PROG=${0##*/}

redact_line() {
    # Same classes and order as tools/probes/rpi5_display/verdicts.sh.
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

[ $# -eq 1 ] || {
    echo "usage: $PROG <console.log>" >&2
    exit 2
}
LOG=$1
if [ ! -r "$LOG" ]; then
    say "$PROG: cannot read the console log: $LOG" >&2
    exit 2
fi

TMP=$(mktemp "${TMPDIR:-/tmp}/verdicts16.XXXXXX") || exit 2
trap 'rm -f "$TMP"' EXIT
tr -d '\r' < "$LOG" > "$TMP"

N=11
KEYS=(
    "u3 scanout"
    "u3 plane"
    "u3 adopt-readback"
    "rpi5_scanout16 text-rows"
    "rpi5_scanout16 pattern"
    "u3 readback 1"
    "u3 readback 2"
    "u3 frcnt"
    "u3 pv"
    "u3 hold"
    "rpi5_scanout16 terminal"
)

# pass 1: the first occurrence of each expected line. Key 12 is the
# 'feeding stopped' ending, kept apart so it prints next to a PASS/FAIL line.
declare -A SEEN
while IFS=$'\t' read -r idx line; do
    SEEN[$idx]=$line
done < <(awk '
    BEGIN {
        re[1]  = "^u3 scanout: (running|stopped)( |[ \t]*$)"
        re[2]  = "^u3 plane: "
        re[3]  = "^u3 adopt-readback: (PASS|FAIL)( |[ \t]*$)"
        re[4]  = "^rpi5_scanout16: text rows [0-9]+ changed([ \t]|$)"
        re[5]  = "^rpi5_scanout16: pattern (drawn|not drawn)( |[ \t]*$)"
        re[6]  = "^u3 readback 1: (PASS|FAIL)( |[ \t]*$)"
        re[7]  = "^u3 readback 2: (PASS|FAIL)( |[ \t]*$)"
        re[8]  = "^u3 frcnt: before [0-9]+ after [0-9]+([ \t]|$)"
        re[9]  = "^u3 pv: control=0x[0-9a-fA-F]+ en=[01] viden=[01]([ \t]|$)"
        re[10] = "^u3 hold: [0-9]+ ms plane=(kept|redrawn) bytes=[0-9]+ list=(kept|changed) words=[0-9]+([ \t]|$)"
        re[11] = "^rpi5_scanout16: (PASS|FAIL|feeding stopped, reset expected)"
        re[12] = "^rpi5_scanout16: feeding stopped, reset expected"
    }
    { sub(/\r$/, "") }
    { for (k = 1; k <= 12; k++) if (!seen[k] && $0 ~ re[k]) { seen[k] = 1; print k "\t" $0 } }
' "$TMP")

nseen=0
LASTKEY=0
for i in $(seq 1 $N); do
    [ -n "${SEEN[$i]:-}" ] && { nseen=$((nseen + 1)); LASTKEY=$i; }
done

say "$PROG: $nseen of $N expected verdict lines in $LOG"
for i in $(seq 1 $N); do
    [ -n "${SEEN[$i]:-}" ] && say "  [$i] ${SEEN[$i]}"
done
if [ -n "${SEEN[12]:-}" ] && [ "${SEEN[12]:-}" != "${SEEN[11]:-}" ]; then
    say "  [12] ${SEEN[12]}"
fi

MISS=""
GAP=""
TAIL=""
for i in $(seq 1 $N); do
    [ -n "${SEEN[$i]:-}" ] && continue
    MISS="$MISS ${KEYS[$((i - 1))]}"
    if [ "$i" -le "$LASTKEY" ]; then GAP="$GAP ${KEYS[$((i - 1))]}"; else TAIL="$TAIL ${KEYS[$((i - 1))]}"; fi
done
if [ -n "$MISS" ]; then
    say "  missing:${MISS}"
    if [ -n "$TAIL" ]; then
        if [ "$LASTKEY" -gt 0 ]; then
            say "  after the last seen line [${LASTKEY}] ${SEEN[$LASTKEY]}: the image stopped there"
        else
            say "  no expected verdict line was seen at all"
        fi
    fi
    [ -n "$GAP" ] && say "  gap: missing between seen lines:${GAP}"
else
    say "  missing: none"
fi

# the Q2 answer, from the hold line alone
if [ -n "${SEEN[10]:-}" ]; then
    h=${SEEN[10]}
    hms=$(grep -oE 'hold: [0-9]+ ms' <<< "$h" | grep -oE '[0-9]+')
    hplane=$(grep -oE 'plane=[a-z]+' <<< "$h" | cut -d= -f2)
    hbytes=$(grep -oE 'bytes=[0-9]+' <<< "$h" | cut -d= -f2)
    hlist=$(grep -oE 'list=[a-z]+' <<< "$h" | cut -d= -f2)
    hwords=$(grep -oE 'words=[0-9]+' <<< "$h" | cut -d= -f2)
    hms=$((10#$hms)); hbytes=$((10#$hbytes)); hwords=$((10#$hwords))
    redrawn=0; changed=0; contra=""
    [ "$hplane" = redrawn ] && redrawn=1
    [ "$hlist" = changed ] && changed=1
    if [ $redrawn = 1 ] && [ $hbytes -eq 0 ]; then contra="plane=redrawn with bytes=0"; fi
    if [ $redrawn = 0 ] && [ $hbytes -ne 0 ]; then contra="plane=kept with bytes=$hbytes"; fi
    if [ $changed = 1 ] && [ $hwords -eq 0 ]; then contra="list=changed with words=0"; fi
    if [ $changed = 0 ] && [ $hwords -ne 0 ]; then contra="list=kept with words=$hwords"; fi
    if [ $hms -eq 0 ]; then
        say "q2: unclear (a hold of 0 ms holds nothing)"
    elif [ -n "$contra" ]; then
        say "q2: unclear ($contra)"
    elif [ $redrawn = 1 ] && [ $changed = 1 ]; then
        say "q2: yes, redrawn and list changed ($hbytes plane bytes, $hwords list words in $hms ms)"
    elif [ $redrawn = 1 ]; then
        say "q2: yes, redrawn ($hbytes plane bytes changed in $hms ms)"
    elif [ $changed = 1 ]; then
        say "q2: yes, list changed ($hwords list words changed in $hms ms)"
    else
        say "q2: no (the firmware neither redrew the plane nor changed its list in $hms ms)"
    fi
else
    say "q2: unanswered (no hold line)"
fi

if [ "$nseen" -ne "$N" ]; then
    say "verdicts: incomplete (the run did not print all $N expected verdict lines)"
    exit 1
fi

# the verdict, from the evidence lines
REASONS=""
add() { REASONS="${REASONS:+$REASONS; }$1"; }
res_of() { grep -oE 'PASS|FAIL' <<< "$1" | head -n 1; }
[ "$(res_of "${SEEN[3]}")" = PASS ] || add "the plane is not adopted (adopt-readback FAIL)"
[ "$(res_of "${SEEN[6]}")" = PASS ] || add "readback 1 FAIL"
[ "$(res_of "${SEEN[7]}")" = PASS ] || add "readback 2 FAIL"

fb=$(grep -oE 'before [0-9]+' <<< "${SEEN[8]}" | grep -oE '[0-9]+')
fa=$(grep -oE 'after [0-9]+' <<< "${SEEN[8]}" | grep -oE '[0-9]+')
fb=$((10#$fb)); fa=$((10#$fa))
if [ "$fb" -gt 63 ] || [ "$fa" -gt 63 ]; then
    add "frcnt $fb -> $fa is not a 6-bit counter"
elif [ $(((fa - fb + 64) % 64)) -eq 0 ]; then
    add "frcnt did not advance ($fb -> $fa)"
fi

pvline=${SEEN[9]}
[[ "$pvline" == *" en=1 "* ]] || add "PV_CONTROL.EN is 0"
[[ "$pvline" == *" viden=1"* ]] || add "PV_V_CONTROL.VIDEN is 0"

term=$(grep -oE 'rpi5_scanout16: (PASS|FAIL)' <<< "${SEEN[11]}" | cut -d' ' -f2)
[ "$term" = FAIL ] && add "the probe's own result line says FAIL"

if [ -z "$REASONS" ]; then
    say "verdicts: PASS (plane adopted, both read-backs match, frcnt $fb -> $fa advanced, EN and VIDEN set)"
    exit 0
fi
say "verdicts: FAIL ($REASONS)"
exit 3
