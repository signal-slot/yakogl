#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/verdicts.sh <console.log> [linux_ref.txt]
#
# Reads the console log of the rpi5_display probe (the U3 board session on
# the Raspberry Pi 5, driver/docs/rpi5-display.md) and names, on one line,
# the display route the board's facts support:
#
#   route A   the firmware's running scanout can be adopted:
#             'u3 scanout: running' and 'u3 adopt-readback: PASS'
#   route B   the BCM2712 property mailbox framebuffer works:
#             'u3 mailbox-fb: PASS (...)' and 'u3 mailbox-readback: PASS'
#   route C   every expected line is there and neither pair passes
#   incomplete an expected line is missing (completeness below)
#
# A complete log holds these lines, in this order, once each (other log
# lines are ignored):
#
#   u3 scanout: running|stopped ...
#   u3 plane: addr=0x<hex> fmt=<f> pitch=<n> w=<n> h=<n>
#   u3 adopt-readback: PASS|FAIL ...
#   u3 mailbox-fb: PASS (w x h pitch <n> <fmt>)|FAIL (reason)
#   u3 mailbox-readback: PASS|FAIL ...
#   u3 scanout-follows-mailbox: yes|no ...
#   u3 split-screen: drawn ...
#   rpi5_display: PASS (...)|FAIL (reason)|feeding stopped, reset expected
#
# The last line accepts either a result line or the planned watchdog
# ending. 'rpi5_display: feeding stopped, reset expected' following a
# PASS/FAIL line is extracted and printed too, and is not required. The
# plane line accepts its five fields in any order. A probe should print
# every line, even when an earlier step failed: a skipped line makes the
# log incomplete even when a result line closed the run.
#
# Missing expected lines are named. Missing lines after the last seen one
# mean the image stopped there; missing lines between two seen ones are a
# gap. An incomplete log gets no route: what a run that did not finish
# says about the routes is not a verdict, so the route line reads
# 'incomplete' and the exit is 1, even when the seen lines already
# exclude routes A and B.
#
# With linux_ref.txt (the 'key: value' output of linux_ref.sh in this
# directory), the plane fields of the log (fmt, pitch, w, h) are compared
# with the facts whose value names a framebuffer or plane fact: one
# carrying fmt= or format=, pitch= or stride=, w= or h=, a WxH pair, a
# four-digit number or a word. Keys that name session plumbing (the
# linux_ref lines itself, get_throttled, kernel, cmdline, config, serial,
# host, board, time) are not facts; a value holding 0x is never a word or a
# number hit, so bare register dumps carry nothing to compare and are
# not counted. 'fmt' matches are case-insensitive only: the firmware and
# Linux name the same buffer differently (ARGB8888 vs xrgb8888) and that
# reads as a differ. The 'linux_ref: done (N facts, M skipped)' line is
# checked for presence only. The comparison is information: it never
# changes the route or the exit status.
#
# Everything the script prints goes through a redactor first, like
# tools/board.sh's redact_line: IPv4 and IPv6 addresses, MAC addresses,
# host:port pairs, /dev/tty* and /dev/serial* paths and ttyS/ttyUSB/
# ttyACM/ttyAMA names come out as markers; rules run twice so a second
# value next to an already masked one is caught too. A DRAM address is
# not an address class: the 'u3 plane' line prints its addr=0x...
# unredacted, as do other 0x... values (a mailbox tag, a register value).
#
# Exit status:
#   0  complete: a route is named (A, B or C)
#   1  incomplete: an expected verdict line is missing
#   2  usage error (argument count, an unreadable file)
#
# Self-test: tools/probes/rpi5_display/verdicts_selftest.sh
set -u

PROG=${0##*/}

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

field_value() { # field_value <line> <field regex> -- the first match
    grep -oE "$2" <<< "$1" | head -n 1
}

[ $# -eq 1 ] || [ $# -eq 2 ] || {
    echo "usage: $PROG <console.log> [linux_ref.txt]" >&2
    exit 2
}
LOG=$1
REF=${2:-}
if [ ! -r "$LOG" ]; then
    say "$PROG: cannot read the console log: $LOG" >&2
    exit 2
fi
if [ -n "$REF" ] && [ ! -r "$REF" ]; then
    say "$PROG: cannot read the linux_ref file: $REF" >&2
    exit 2
fi

TMP=$(mktemp "${TMPDIR:-/tmp}/verdicts.XXXXXX") || exit 2
trap 'rm -f "$TMP"' EXIT
tr -d '\r' < "$LOG" > "$TMP"

# pass 1: the first occurrence of each expected line, in key order.
# Key 9 is the 'feeding stopped' ending, kept apart so it prints next to
# a PASS/FAIL line without being counted twice.
declare -A SEEN
KEYS=(
    "u3 scanout"
    "u3 plane"
    "u3 adopt-readback"
    "u3 mailbox-fb"
    "u3 mailbox-readback"
    "u3 scanout-follows-mailbox"
    "u3 split-screen"
    "rpi5_display terminal"
)
while IFS=$'\t' read -r idx line; do
    SEEN[$idx]=$line
done < <(awk '
    { sub(/\r$/, "") }
    !seen[1] && /^u3 scanout: (running|stopped)( |[ \t]*$)/ { seen[1] = 1; print 1 "\t" $0 }
    !seen[2] && /^u3 plane: / { seen[2] = 1; print 2 "\t" $0 }
    !seen[3] && /^u3 adopt-readback: (PASS|FAIL)( |[ \t]*$)/ { seen[3] = 1; print 3 "\t" $0 }
    !seen[4] && /^u3 mailbox-fb: (PASS|FAIL)( |[ \t]*$)/ { seen[4] = 1; print 4 "\t" $0 }
    !seen[5] && /^u3 mailbox-readback: (PASS|FAIL)( |[ \t]*$)/ { seen[5] = 1; print 5 "\t" $0 }
    !seen[6] && /^u3 scanout-follows-mailbox: (yes|no)( |[ \t]*$)/ { seen[6] = 1; print 6 "\t" $0 }
    !seen[7] && /^u3 split-screen: / { seen[7] = 1; print 7 "\t" $0 }
    !seen[8] && /^rpi5_display: (PASS|FAIL|feeding stopped, reset expected)/ { seen[8] = 1; print 8 "\t" $0 }
    !seen[9] && /^rpi5_display: feeding stopped, reset expected/ { seen[9] = 1; print 9 "\t" $0 }
' "$TMP")

nseen=0
LASTKEY=0
for i in 1 2 3 4 5 6 7 8; do
    [ -n "${SEEN[$i]:-}" ] && { nseen=$((nseen + 1)); LASTKEY=$i; }
done

say "$PROG: $nseen of 8 expected verdict lines in $LOG"
for i in 1 2 3 4 5 6 7 8; do
    [ -n "${SEEN[$i]:-}" ] && say "  [$i] ${SEEN[$i]}"
done
if [ -n "${SEEN[9]:-}" ] && [ "${SEEN[9]:-}" != "${SEEN[8]:-}" ]; then
    say "  [9] ${SEEN[9]}"
fi

# missing lines: a tail after the last seen one means the image stopped
# there; missing ones between two seen ones are a gap (skipped lines)
MISS=""
GAP=""
TAIL=""
for i in 1 2 3 4 5 6 7 8; do
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

# the four route lines carry the values the verdicts are named from
VAL1=$(field_value "${SEEN[1]:-}" 'PASS|FAIL|running|stopped')
VAL3=$(field_value "${SEEN[3]:-}" 'PASS|FAIL|running|stopped')
VAL4=$(field_value "${SEEN[4]:-}" 'PASS|FAIL|running|stopped')
VAL5=$(field_value "${SEEN[5]:-}" 'PASS|FAIL|running|stopped')
A_OK=""
B_OK=""
[ "$VAL1" = "running" ] && [ "$VAL3" = "PASS" ] && A_OK=1
[ "$VAL4" = "PASS" ] && [ "$VAL5" = "PASS" ] && B_OK=1

if [ "$nseen" -eq 8 ]; then
    if [ -n "$A_OK" ]; then
        say "verdicts: route A (the firmware scanout runs and the adopt read-back matches)"
    elif [ -n "$B_OK" ]; then
        say "verdicts: route B (the mailbox allocated a framebuffer and the read-back matches)"
    else
        say "verdicts: route C (neither the firmware scanout nor the mailbox path works)"
    fi
    rc=0
else
    say "verdicts: incomplete (the run did not print all eight expected verdict lines)"
    rc=1
fi

# optional: compare the log's plane with the linux_ref framebuffer/plane
# facts. Information only: it never changes the route or the exit status.
if [ -n "$REF" ]; then
    plane_line=${SEEN[2]:-}
    if [ -z "$plane_line" ]; then
        say "verdicts: linux_ref: the log has no u3 plane line, nothing compared"
    else
        pfmt=$(field_value "$plane_line" '\bfmt=[^[:space:]]+' | sed -E 's/^fmt=//')
        ppitch=$(field_value "$plane_line" '\bpitch=[[:digit:]]+' | sed -E 's/^pitch=//')
        pw=$(field_value "$plane_line" '\bw=[[:digit:]]+' | sed -E 's/^w=//')
        ph=$(field_value "$plane_line" '\bh=[[:digit:]]+' | sed -E 's/^h=//')
        grep -Eq '^linux_ref: done \([[:digit:]]+ facts, [[:digit:]]+ skipped\)[[:space:]]*$' "$REF" \
            && say "verdicts: linux_ref: the done line is there" \
            || say "verdicts: linux_ref: no done line (the dump may be cut)"
        compared=0 agree=0 differ=0
        while IFS= read -r kv; do
            key=${kv%%:*}
            value=${kv#*:}
            rfmt=$(field_value "$value" '\bfmt=[^[:space:]]+|\bformat=[^[:space:]]+' | sed -E 's/^(fmt|format)=//')
            rpitch=$(field_value "$value" '\bpitch=[[:digit:]]+|\bstride=[[:digit:]]+' | sed -E 's/^(pitch|stride)=//')
            rw=$(field_value "$value" '\bw=[[:digit:]]+' | sed -E 's/^w=//')
            rh=$(field_value "$value" '\bh=[[:digit:]]+' | sed -E 's/^h=//')
            if [ -z "$rw" ] || [ -z "$rh" ]; then
                pair=$(field_value "$value" '[[:digit:]]{2,5}[xX][[:digit:]]{2,5}')
                [ -z "$rw" ] && rw=${pair%%[xX]*}
                [ -z "$rh" ] && rh=${pair##*[xX]}
            fi
            fields=""
            if [ -n "$rfmt" ] && [ -n "$pfmt" ]; then
                if [ "${rfmt,,}" = "${pfmt,,}" ]; then fields="$fields fmt-agree"
                else fields="$fields fmt-differ($rfmt/$pfmt)"; fi
            fi
            if [ -n "$rpitch" ] && [ -n "$ppitch" ]; then
                if [ "$rpitch" = "$ppitch" ]; then fields="$fields pitch-agree"
                else fields="$fields pitch-differ($rpitch/$ppitch)"; fi
            fi
            if [ -n "$rw" ] && [ -n "$pw" ]; then
                if [ "$rw" = "$pw" ]; then fields="$fields w-agree"
                else fields="$fields w-differ($rw/$pw)"; fi
            fi
            if [ -n "$rh" ] && [ -n "$ph" ]; then
                if [ "$rh" = "$ph" ]; then fields="$fields h-agree"
                else fields="$fields h-differ($rh/$ph)"; fi
            fi
            [ -z "$fields" ] && continue
            compared=$((compared + 1))
            if [[ "$fields" == *differ* ]]; then differ=$((differ + 1)); else agree=$((agree + 1)); fi
            say "  ref [$key]:${fields}"
        done < <(grep -Ei '^[^:]+: .*(framebuffer|plane|pitch|stride|fmt|format|[[:digit:]]{2,5}[xX][[:digit:]]{2,5}|[[:digit:]]{4,}|[a-z]{4,})' "$REF" \
            | grep -Eiv '^(linux_ref|get_throttled|kernel|cmdline|config|serial|host|board|time)[^ ]*: ')
        if [ "$compared" -eq 0 ]; then
            say "verdicts: linux_ref: no framebuffer or plane fact to compare"
        else
            say "verdicts: linux_ref: $compared fact(s) compared, $agree agree, $differ differ"
        fi
    fi
fi

exit "$rc"
