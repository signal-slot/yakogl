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
#   u3 snapshot: list-ptr=0x<hex> heade=<n> words=<n> hash=0x<hex>
#       the firmware's state before anything is changed: channel 0's list
#       pointer, heade = the firmware list's start word (LPTRS.HEADE, read at
#       run time, decimal), words = its length in words up to and including
#       its END word (decimal, >= 1) and a hash of those words. The firmware's
#       span is [heade, heade+words). A snapshot without heade= is malformed
#       (the line counts as missing)
#   u3 dlist: placed word <n> len <n>
#     | u3 dlist: refused <free text>
#       where our list starts in the list memory and its length in words.
#       Valid only if len >= 1, [word, word+len) does not overlap
#       [heade, heade+words) and word+len <= 4096 (the 12-bit LPTRS field).
#       Words below heade are not assumed used or unused: the probe places
#       only in a span it proved unused. A probe that finds none prints the
#       'refused' line instead; the swap, plane, named and restore lines are
#       then not required (plane, swap and named are shown if present)
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
#       first swap and DL.LACT picked it up (the probe says yes only then)
#   u3 restore: PASS|FAIL <free text>
#       the firmware's list words and pointer byte-equal to the snapshot.
#       A PASS carrying hash=0x<hex> must equal the snapshot's hash; a
#       heade=<n> on the line, if present, must equal the snapshot's heade
#   rpi5_scanout32: PASS (...)|FAIL (reason)|feeding stopped, reset expected
#
# 'rpi5_scanout32: feeding stopped, reset expected' after a PASS/FAIL line
# is printed too and is not required. A probe prints every line even when
# a step failed: a skipped line makes the log incomplete.
#
# DL.LACT pickup (lact-probe-a; hvs6_dlist_swap.h, swap-lact-lib). A swap
# counts only once channel 0's DISPx_DL.LACT read the head just written, a
# restore only once DL.LACT is back on the firmware's LPTRS.HEADE. Lines,
# anywhere in the log:
#
#   dlswap lact-pickup ok|FAIL polls=<n> frcnt=<a>..<b> want=<w> lact=<l>
#       one per swap made, in swap order: the k-th is swap k's (swap 1's is
#       in hvs6s_run's record, the others follow their dlswap swap<k> line)
#   dlswap restore-lact ok|FAIL polls=<n> frcnt=<a>..<b> want=<w> lact=<l>
#       one per restore (swap 1's own restore, then the final one)
#   dlswap swap<k> FAIL DL.LACT did not reach the new head ...
#   ... restore: DL.LACT not back on the firmware's head ...
#       hvs6_dlist_swap.h's two error names (HVS6S_ERR_LACT_PICKUP,
#       HVS6S_ERR_RESTORE_LACT); the first stands for a swap's missing
#       lact-pickup line, both name the cause of a 'u3 restore: FAIL'
#
# A swap with no lact-pickup line or a FAIL one fails q4 (q4 is never 'yes'
# without every pickup) and the verdict; a 'u3 restore: PASS' needs every
# pickup ok and at least one restore-lact line, all ok. A probe stops at the
# first failed swap: when the last of fewer than MIN_SWAPS swap lines failed
# its pickup, or the only swap line is followed by swap 1's restore-lact
# FAIL, the log is judged (FAIL, q4 no), not incomplete. So is a stop at a
# swap that failed another way: the last of fewer than MIN_SWAPS swap lines
# has a 'dlswap swap<k> FAIL <error name> ...' line (FRCNT, a read-back,
# swap 1's restore), or the only swap line comes with a 'dlswap
# restore-frcnt|restore-lptrs|restore-words FAIL' line of swap 1's restore.
# A stop with no failed swap or restore line (slot B refused) stays
# incomplete.
#
# Vblank landing (swap-lact-probe; hvs6_dlist_swap.h, swap-vblank-lib). Each
# swap writes LPTRS just after an FRCNT step of channel 0's STATUS found by
# reads, and times the DL.LACT pickup with a tight loop. Lines, anywhere in
# the log:
#
#   u3 vblank <k>: landed=yes|no why=<reason> frcnt <w> -> <p> steps=<n>
#       invblank=yes|no tight=ok|miss pickup_reads=<n> pickup_us=<us|none>
#       read_us=<us|none> boundary=yes|no since_us=<us|none> yline <a> -> <b>
#       (yline record only)
#       one per swap made, k its number (printed after 'u3 swap <k>'); w and
#       p FRCNT at the write and at the pickup (6-bit), invblank the write
#       right after the step, tight=ok the pickup inside the tight loop
#   dlswap vblank ok|FAIL mode=... / dlswap lact-timing tight=... (the
#       library's record of the same, one per swap, shown, not judged)
#   dlswap swap<k> FAIL no FRCNT step (vblank) within the read bound ...
#       HVS6S_ERR_VBLANK: LPTRS was not written; the probe stops there
#
# Swap k landed when its line says landed=yes, invblank=yes, tight=ok and
# FRCNT stepped at most once from w to p ((p - w) & 63 <= 1). 'q4: yes' and
# the PASS verdict need every swap landed; a swap without its line (an older
# probe) or one that did not land makes q4 'no', naming the first: a missed
# frame (more than one step), a pickup timeout (tight=miss), a write outside
# the vblank, or no vblank found. boundary=, pickup_us=, read_us= and YLINE
# are printed as a record and decide nothing. A stop at a swap whose vblank
# wait found no FRCNT step is judged (FAIL, q4 no), not incomplete.
#
# Output: the seen lines, 'missing:' naming absent lines (after the last
# seen one: the image stopped there; between two seen ones: a gap), then
#   verdicts: PASS (...)            all of: placement valid (no overlap with
#                                   the firmware span, within 4096), our list
#                                   named, every swap's FRCNT advanced and
#                                   DL.LACT picked it up, every swap landed
#                                   in vblank, the buffers
#                                   alternated, restore PASS with DL.LACT
#                                   back, the probe's own line not FAIL
#   verdicts: FAIL (reasons)        complete log, one of those not met
#   verdicts: FAIL (placement refused: <text>) for a 'refused' line, with
#   q4: unanswered (placement refused)
#   verdicts: incomplete (...)      a line missing (fewer than MIN_SWAPS
#                                   swap lines count as missing 'u3 swap')
#   q4: yes|no|unanswered (...)     can channel 0's list pointer be swapped
#                                   at vblank under a running scanout:
#                                   from the swap lines, 'yes' needs the
#                                   swaps, FRCNT, every DL.LACT pickup,
#                                   every swap landed in vblank and
#                                   naming to hold (the
#                                   restore is judged separately), 'no'
#                                   names the first swap that failed,
#                                   'unanswered' for an incomplete log
# Everything printed goes through the redactor first (IPv4/IPv6, MAC,
# host:port, tty and serial device paths, S/USB/ACM/AMA tty names); 0x values
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
    !seen[1] && /^u3 snapshot: list-ptr=0x[[:xdigit:]]+ heade=[0-9]+ words=[0-9]+ hash=0x[[:xdigit:]]+/ { seen[1] = 1; print 1 "\t" $0 }
    !seen[2] && (/^u3 dlist: placed word [0-9]+ len [0-9]+/ || /^u3 dlist: refused( |[ \t]*$)/) { seen[2] = 1; print 2 "\t" $0 }
    !seen[3] && /^u3 plane: / { seen[3] = 1; print 3 "\t" $0 }
    /^u3 swap [0-9]+: buf [AB] list-ptr=0x[[:xdigit:]]+ frcnt [0-9]+ -> [0-9]+/ { print "s\t" $0 }
    !seen[5] && /^u3 named: (yes|no)( |[ \t]*$)/ { seen[5] = 1; print 5 "\t" $0 }
    !seen[6] && /^u3 restore: (PASS|FAIL)( |[ \t]*$)/ { seen[6] = 1; print 6 "\t" $0 }
    !seen[7] && /^rpi5_scanout32: (PASS|FAIL|feeding stopped, reset expected)/ { seen[7] = 1; print 7 "\t" $0 }
    !seen[8] && /^rpi5_scanout32: feeding stopped, reset expected/ { seen[8] = 1; print 8 "\t" $0 }
' "$TMP")
NSW=${#SW[@]}
[ "$NSW" -ge "$MIN_SWAPS" ] && SEEN[4]=${SW[0]}

# DL.LACT pickup lines (lact-probe-a): LP[j] is swap j+1's, RL every restore's.
LP=() RL=()
while IFS= read -r l; do LP+=("$l"); done < <(grep -E '^dlswap lact-pickup (ok|FAIL) polls=' "$TMP")
while IFS= read -r l; do RL+=("$l"); done < <(grep -E '^dlswap restore-lact (ok|FAIL) polls=' "$TMP")
ERR_PICKUP="DL.LACT did not reach the new head"
ERR_RLACT="restore: DL.LACT not back on the firmware's head"
# swap k's pickup: ok|FAIL|"" (no line); the line, or the swap<k> FAIL line
pickup_of() {
    local j=$1 k=$2 l=${LP[$1]:-}
    if [ -n "$l" ]; then
        PU_LINE=$l
        PU=$(sed -E 's/^dlswap lact-pickup (ok|FAIL) .*/\1/' <<< "$l")
    elif l=$(grep -m 1 -F "dlswap swap$k FAIL $ERR_PICKUP" "$TMP"); then
        PU_LINE=$l PU=FAIL
    else
        PU_LINE="" PU=""
    fi
}
# Vblank lines (swap-lact-probe): VB[k] is swap k's 'u3 vblank <k>:' line
# (the first one per k), DV the library's dlswap vblank/lact-timing records.
declare -A VB=()
DV=()
while IFS= read -r l; do
    k=$(sed -E 's/^u3 vblank ([0-9]+):.*/\1/' <<< "$l")
    [ -z "${VB[$k]:-}" ] && VB[$k]=$l
done < <(grep -E '^u3 vblank [0-9]+: landed=(yes|no) ' "$TMP")
while IFS= read -r l; do DV+=("$l"); done < <(grep -E '^dlswap (vblank (ok|FAIL) mode=|lact-timing tight=)' "$TMP")
ERR_VBLANK="no FRCNT step (vblank) within the read bound"
# swap k found no vblank: LPTRS was not written (HVS6S_ERR_VBLANK)
vb_nostep() {
    grep -qF "dlswap swap$1 FAIL $ERR_VBLANK" "$TMP" && return 0
    case "${VB[$1]:-}" in *" why=no-vblank "*) return 0 ;; esac
    return 1
}
lact_vals() { # "want=W lact=L" of a lact line; a dlswap swap<k> line has none
    local w
    w=$(grep -oE 'want=[0-9]+ lact=[0-9]+' <<< "$1" | head -n 1)
    printf '%s' "${w:-named on its dlswap swap line}"
}
# The probe stops at the first failed swap, and after swap 1 when swap 1's
# own restore did not bring DL.LACT back: fewer than MIN_SWAPS swap lines
# whose last failed its pickup, or one swap line with swap 1's restore-lact
# FAIL, are a finished run, judged below.
STOPPED=0 STOP_WHY=""
if [ "$NSW" -ge 1 ] && [ "$NSW" -lt "$MIN_SWAPS" ]; then
    kl=$(sed -E 's/^u3 swap ([0-9]+):.*/\1/' <<< "${SW[$((NSW - 1))]}")
    pickup_of $((NSW - 1)) "$kl"
    if vb_nostep "$kl"; then
        STOPPED=$kl STOP_WHY="swap $kl's vblank wait ($ERR_VBLANK)"
    elif [ "$PU" = FAIL ]; then
        STOPPED=$kl STOP_WHY="swap $kl's failed DL.LACT pickup"
    elif [ "$NSW" -eq 1 ] && [ "$PU" = ok ] && [[ "${RL[0]:-}" == "dlswap restore-lact FAIL "* ]]; then
        STOPPED=1 STOP_WHY="swap 1's restore: DL.LACT not back on the firmware's head"
    elif l=$(grep -m 1 -E "^dlswap swap$kl FAIL " "$TMP"); then
        # (gap 3) the swap failed another way (FRCNT, a read-back, swap 1's
        # restore): its dlswap swap<k> line names the error
        n=$(sed -E "s/^dlswap swap$kl FAIL (.*) lptrs-readback .*/\1/; s/^dlswap swap$kl FAIL //" <<< "$l")
        case "$n" in
        "restore: "*) STOPPED=$kl STOP_WHY="swap $kl's $n" ;;
        *) STOPPED=$kl STOP_WHY="swap $kl's failure ($n)" ;;
        esac
    elif [ "$NSW" -eq 1 ] && l=$(grep -m 1 -E '^dlswap restore-(frcnt|lptrs|words) FAIL' "$TMP"); then
        # (gap 3) swap 1's own restore failed other than DL.LACT, with no
        # dlswap swap1 line naming it
        n=$(sed -E 's/^dlswap (restore-[a-z]+) FAIL.*/\1/' <<< "$l")
        STOPPED=1 STOP_WHY="swap 1's restore: dlswap $n FAIL"
    fi
    [ "$STOPPED" -ne 0 ] && SEEN[4]=${SW[0]}
fi

REFUSED=0
case "${SEEN[2]:-}" in "u3 dlist: refused"*) REFUSED=1 ;; esac
# a refused placement needs only the snapshot, the dlist line and the
# probe's own line; plane, swap, named and restore are then optional
if [ "$REFUSED" -eq 1 ]; then REQ="1 2 7"; else REQ="1 2 3 4 5 6 7"; fi
isreq() { case " $REQ " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
nreq=$(wc -w <<< "$REQ")
nseen=0
LASTKEY=0
for i in 1 2 3 4 5 6 7; do
    [ -n "${SEEN[$i]:-}" ] && { nseen=$((nseen + 1)); LASTKEY=$i; }
done
# a few swap lines (fewer than MIN_SWAPS) still show where the image stopped
if [ "$NSW" -gt 0 ] && [ "$LASTKEY" -lt 4 ]; then LASTKEY=4; fi
LASTLINE=${SEEN[$LASTKEY]:-}
if [ "$LASTKEY" -eq 4 ] && [ "$NSW" -gt 0 ]; then LASTLINE=${SW[$((NSW - 1))]}; fi

say "$PROG: $nseen of $nreq expected verdict lines in $LOG ($NSW swap line(s))"
for i in 1 2 3; do [ -n "${SEEN[$i]:-}" ] && say "  [$i] ${SEEN[$i]}"; done
for ((j = 0; j < NSW; j++)); do say "  [4] ${SW[$j]}"; done
for l in ${LP[@]+"${LP[@]}"} ${RL[@]+"${RL[@]}"}; do say "  [l] $l"; done
for l in ${DV[@]+"${DV[@]}"}; do say "  [v] $l"; done
for ((j = 0; j < NSW; j++)); do
    k=$(sed -E 's/^u3 swap ([0-9]+):.*/\1/' <<< "${SW[$j]}")
    [ -n "${VB[$k]:-}" ] && say "  [v] ${VB[$k]}"
done
for i in 5 6 7; do [ -n "${SEEN[$i]:-}" ] && say "  [$i] ${SEEN[$i]}"; done
if [ -n "${SEEN[8]:-}" ] && [ "${SEEN[8]}" != "${SEEN[7]:-}" ]; then say "  [8] ${SEEN[8]}"; fi

MISS="" GAP="" TAIL=""
for i in 1 2 3 4 5 6 7; do
    [ -n "${SEEN[$i]:-}" ] && continue
    isreq "$i" || continue
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

miss_req=0
for i in $REQ; do [ -z "${SEEN[$i]:-}" ] && miss_req=1; done
if [ "$miss_req" -eq 1 ]; then
    say "verdicts: incomplete (the run did not print all expected verdict lines)"
    say "q4: unanswered (incomplete log)"
    exit 1
fi

# ---- complete log: judge ---------------------------------------------------
heade=$(sed -E 's/.* heade=([0-9]+) .*/\1/' <<< "${SEEN[1]}")
fwlen=$(sed -E 's/.* words=([0-9]+) .*/\1/' <<< "${SEEN[1]}")
snap_bad=""
[ "$fwlen" -ge 1 ] || snap_bad="snapshot words=$fwlen (must be >= 1)"
if [ "$REFUSED" -eq 1 ]; then
    txt=$(sed -E 's/^u3 dlist: refused[ \t]*//' <<< "${SEEN[2]}")
    r="placement refused: $txt"
    [ -n "$snap_bad" ] && r="$r; $snap_bad"
    say "q4: unanswered (placement refused)"
    say "verdicts: FAIL ($r)"
    exit 3
fi
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

# ---- DL.LACT pickup per swap (lact-probe-a) -----------------------------------
pick_bad=0 pick1=""
for ((j = 0; j < NSW; j++)); do
    k=$(sed -E 's/^u3 swap ([0-9]+):.*/\1/' <<< "${SW[$j]}")
    pickup_of "$j" "$k"
    [ "$j" -eq 0 ] && pick1=$PU
    if vb_nostep "$k"; then
        # LPTRS was not written: the missing pickup follows from it, and
        # the vblank section below names the cause
        pick_bad=1
    elif [ -z "$PU" ]; then
        pick_bad=1
        [ -z "$swap_fail" ] && swap_fail="swap $k: no dlswap lact-pickup line, DL.LACT pickup not shown"
        other+=("swap $k: no dlswap lact-pickup line (DL.LACT pickup not shown)")
    elif [ "$PU" != ok ]; then
        pick_bad=1
        v=$(lact_vals "$PU_LINE")
        [ -z "$swap_fail" ] && swap_fail="swap $k: $ERR_PICKUP ($v)"
        other+=("$ERR_PICKUP at swap $k ($v)")
    fi
done

# ---- vblank landing per swap (swap-lact-probe) ---------------------------------
# From the u3 vblank line's fields, in the probe's order (swap_vblank_judge):
# vblank found, written in it, picked up in the tight loop, at most one FRCNT
# step; then the line's own landed=. YLINE, boundary= and the times are shown
# only.
for ((j = 0; j < NSW; j++)); do
    k=$(sed -E 's/^u3 swap ([0-9]+):.*/\1/' <<< "${SW[$j]}")
    l=${VB[$k]:-}
    why=""
    if vb_nostep "$k"; then
        why="swap $k: no vblank found ($ERR_VBLANK), LPTRS not written"
    elif [ -z "$l" ]; then
        why="swap $k: no u3 vblank line, vblank landing not shown"
    elif ! grep -qE ' frcnt [0-9]+ -> [0-9]+ .*invblank=(yes|no) tight=(ok|miss) ' <<< "$l"; then
        why="swap $k: malformed u3 vblank line"
    else
        landed=$(sed -E 's/.* landed=(yes|no) .*/\1/' <<< "$l")
        vwhy=$(sed -nE 's/.* why=([^ ]+) .*/\1/p' <<< "$l")
        w=$(sed -E 's/.* frcnt ([0-9]+) -> ([0-9]+) .*/\1/' <<< "$l")
        p=$(sed -E 's/.* frcnt ([0-9]+) -> ([0-9]+) .*/\2/' <<< "$l")
        inv=$(sed -E 's/.* invblank=(yes|no) .*/\1/' <<< "$l")
        tight=$(sed -E 's/.* tight=(ok|miss) .*/\1/' <<< "$l")
        if [ "$w" -gt 63 ] || [ "$p" -gt 63 ]; then
            why="swap $k: vblank frcnt out of the 6-bit range ($w -> $p)"
        elif [ "$inv" != yes ]; then
            why="swap $k: written outside the vblank (invblank=no)"
        elif [ "$tight" != ok ]; then
            why="swap $k: DL.LACT pickup timeout (the tight loop ran out, tight=miss)"
        else
            steps=$(( (p - w) & 63 ))
            if [ "$steps" -gt 1 ]; then
                why="swap $k: missed a frame (frcnt $w -> $p, $steps steps from the write to the pickup)"
            elif [ "$landed" != yes ]; then
                why="swap $k: did not land in vblank (why=${vwhy:-none})"
            fi
        fi
        if [ -n "$why" ] && [ "$landed" = yes ]; then
            other+=("u3 vblank $k: landed=yes although it did not land")
        fi
    fi
    if [ -n "$why" ]; then
        [ -z "$swap_fail" ] && swap_fail=$why
        other+=("$why")
    fi
done
if [ "$STOPPED" -ne 0 ]; then
    [ -z "$swap_fail" ] && swap_fail="$STOP_WHY; the probe stopped after swap $STOPPED"
    other+=("$NSW of $MIN_SWAPS swaps made: the probe stopped at $STOP_WHY")
fi

named=$(grep -oE '(yes|no)' <<< "${SEEN[5]}" | head -n 1)
if [ "$named" != "yes" ]; then
    [ -z "$swap_fail" ] && swap_fail="channel 0 did not name our list after the first swap"
    other+=("our list was not named")
elif [ "$pick1" != ok ]; then
    other+=("u3 named: yes although swap 1's DL.LACT pickup is not ok")
fi

word=$(sed -E 's/^u3 dlist: placed word ([0-9]+) len ([0-9]+).*/\1/' <<< "${SEEN[2]}")
len=$(sed -E 's/^u3 dlist: placed word ([0-9]+) len ([0-9]+).*/\2/' <<< "${SEEN[2]}")
[ "$len" -ge 1 ] || other+=("placed list length 0")
[ -n "$snap_bad" ] && other+=("$snap_bad")
if [ "$word" -lt $((heade + fwlen)) ] && [ $((word + len)) -gt "$heade" ]; then
    other+=("placed [$word,$((word + len))) overlaps the firmware list [$heade,$((heade + fwlen)))")
fi
[ $((word + len)) -le 4096 ] || other+=("placed [$word,$((word + len))) runs past word 4096")

grep -qE '\bfmt=XRGB8888( |$)' <<< "${SEEN[3]}" || other+=("plane fmt is not XRGB8888")

rst=$(grep -oE 'PASS|FAIL' <<< "${SEEN[6]}" | head -n 1)
if [ "$rst" != "PASS" ]; then
    case "${SEEN[6]}" in
    *"$ERR_RLACT"*) other+=("restore FAIL ($ERR_RLACT)") ;;
    *"$ERR_PICKUP"*) other+=("restore FAIL (a swap's $ERR_PICKUP)") ;;
    *) other+=("restore FAIL") ;;
    esac
fi
# every restore's DL.LACT back on the firmware's head
rl_bad=0
for l in ${RL[@]+"${RL[@]}"}; do
    case "$l" in
    "dlswap restore-lact ok "*) ;;
    *) rl_bad=1; other+=("$ERR_RLACT ($(lact_vals "$l"))") ;;
    esac
done
if [ "$rst" = "PASS" ]; then
    if [ "$NSW" -gt 0 ] && [ "${#RL[@]}" -eq 0 ]; then
        other+=("restore PASS without a dlswap restore-lact line (DL.LACT back not shown)")
    fi
    [ "$rl_bad" -eq 1 ] && other+=("restore PASS although DL.LACT did not come back to the firmware's head")
    [ "$pick_bad" -eq 1 ] && other+=("restore PASS although a swap's DL.LACT pickup is not ok")
fi
if [ "$rst" = "PASS" ]; then
    rh=$(grep -oE 'hash=0x[[:xdigit:]]+' <<< "${SEEN[6]}" | head -n 1)
    sh=$(grep -oE 'hash=0x[[:xdigit:]]+' <<< "${SEEN[1]}" | head -n 1)
    if [ -n "$rh" ] && [ "${rh,,}" != "${sh,,}" ]; then
        other+=("restore PASS carries $rh but the snapshot has $sh")
    fi
    rhe=$(grep -oE 'heade=[0-9]+' <<< "${SEEN[6]}" | head -n 1)
    if [ -n "$rhe" ] && [ "$rhe" != "heade=$heade" ]; then
        other+=("restore carries $rhe but the snapshot has heade=$heade")
    fi
fi

# ---- hardware-written words (swap-hwword) ------------------------------------
# The probe may leave declared words of a plane entry out of the byte-equal
# compare (RPI5_SCANOUT32_HW_WORDS, README "Hardware-written words"). These
# lines then carry the set and every differing word:
#
#   u3 snapshot: ... hwskip=<set>       u3 restore: ... hwskip=<set>
#       <set> is 'none' or '<off>:skip|expect' items joined by ','; a line
#       without hwskip= (an older probe) is 'none'. The two must agree.
#   dlswap restore-diff word <n> entry <e> off <k> was 0x.. now 0x.. hw=<m>
#   dlswap readback-diff word <n> entry <e> off <k> was 0x.. now 0x.. hw=<m>
#       one per differing word of the firmware's span at a restore, or of
#       our slot at a read-back; <e>/<k> 'none' for a word in no entry
#   dlswap restore-diffs|readback-diffs total=<n> undeclared=<n> ... hwskip=<set>
#       the count lines (diff lines past the probe's cap are only counted)
#
# A differing word passes only when its offset is in the declared set with
# the same mode (hw=skip|expect); any other differing word is a FAIL naming
# it, also when 'u3 restore:' says PASS, and so is a count line with
# undeclared > 0 that no named line explains. A restore PASS with a
# non-empty set must say 'byte-equal except hw words off <offs>'.
hw_set() { grep -oE '(^| )hwskip=[^ )]+' <<< "$1" | head -n 1 | sed -E 's/^ ?hwskip=//'; }
hws=$(hw_set "${SEEN[6]}")
hwn=$(hw_set "${SEEN[1]}")
[ -n "$hws" ] || hws=none
[ -n "$hwn" ] || hwn=none
declare -A DECL=()
if [ "$hws" != "$hwn" ]; then
    other+=("restore hwskip=$hws but the snapshot has hwskip=$hwn")
fi
if [ "$hws" != none ]; then
    if ! grep -qE '^[0-9]+:(skip|expect)(,[0-9]+:(skip|expect))*$' <<< "$hws"; then
        other+=("malformed hwskip=$hws")
    else
        IFS=, read -r -a hw_items <<< "$hws"
        for it in "${hw_items[@]}"; do DECL[$((10#${it%%:*}))]=${it#*:}; done
    fi
fi
hw_bad_restore=0 hw_bad_readback=0
while IFS= read -r l; do
    kind=$(sed -E 's/^dlswap (restore|readback)-diff .*/\1/' <<< "$l")
    w=$(sed -E 's/.* word ([0-9]+) .*/\1/' <<< "$l")
    e=$(sed -E 's/.* entry ([^ ]+) .*/\1/' <<< "$l")
    o=$(sed -E 's/.* off ([^ ]+) .*/\1/' <<< "$l")
    was=$(sed -nE 's/.* was (0x[[:xdigit:]]+) .*/\1/p' <<< "$l")
    now=$(sed -nE 's/.* now (0x[[:xdigit:]]+) .*/\1/p' <<< "$l")
    m=$(sed -nE 's/.* hw=([a-z]+).*/\1/p' <<< "$l")
    d=""
    [[ "$o" =~ ^[0-9]+$ ]] && d=${DECL[$((10#$o))]:-}
    if [ -z "$d" ] || [ "$m" != "$d" ]; then
        if [ "$kind" = restore ]; then
            hw_bad_restore=1
            what="restore word $w"
        else
            hw_bad_readback=1
            what="our slot word $w"
        fi
        other+=("$what (entry $e off $o) changed $was -> $now, not a declared hardware-written word (hwskip=$hws)")
    fi
done < <(grep -E '^dlswap (restore|readback)-diff word [0-9]+ entry [^ ]+ off [^ ]+ ' "$TMP")
while IFS= read -r l; do
    kind=$(sed -E 's/^dlswap (restore|readback)-diffs .*/\1/' <<< "$l")
    u=$(sed -nE 's/.* undeclared=([0-9]+).*/\1/p' <<< "$l")
    ch=$(hw_set "$l")
    if [ -n "$ch" ] && [ "$ch" != "$hws" ]; then
        other+=("a $kind-diffs line has hwskip=$ch, the restore line hwskip=$hws")
    fi
    if [ "${u:-0}" -gt 0 ]; then
        if [ "$kind" = restore ] && [ "$hw_bad_restore" -eq 0 ]; then
            other+=("a restore-diffs line counts $u undeclared word(s) changed")
            hw_bad_restore=1
        elif [ "$kind" = readback ] && [ "$hw_bad_readback" -eq 0 ]; then
            other+=("a readback-diffs line counts $u undeclared slot word(s) changed")
            hw_bad_readback=1
        fi
    fi
done < <(grep -E '^dlswap (restore|readback)-diffs total=' "$TMP")
if [ "$rst" = "PASS" ]; then
    if [ "$hws" != none ] && ! grep -q 'byte-equal except hw words off ' <<< "${SEEN[6]}"; then
        other+=("restore PASS with hwskip=$hws does not say 'byte-equal except hw words off ...'")
    fi
    if [ "$hws" = none ] && grep -q 'except hw words' <<< "${SEEN[6]}"; then
        other+=("restore PASS leaves hw words out but carries no hwskip set")
    fi
fi
hw_note=""
if [ "$hws" != none ]; then
    hw_note=" except hw words off $(sed -E 's/:(skip|expect)//g' <<< "$hws") (hwskip=$hws)"
fi

grep -qE '^rpi5_scanout32: FAIL' <<< "${SEEN[7]}" && other+=("the probe's own line is FAIL")

if [ -n "$swap_fail" ]; then
    say "q4: no ($swap_fail)"
else
    say "q4: yes (channel 0's list pointer swapped $NSW times under the running scanout, named after the first swap, DL.LACT picked up every swap, FRCNT advanced through every swap, every swap written in the vblank and picked up within one frame step)"
fi

if [ ${#other[@]} -eq 0 ]; then
    say "verdicts: PASS ($NSW swaps alternating, our list named at word $word beside the firmware's at $heade, DL.LACT picked up each swap and came back, each swap landed in vblank within one frame step, FRCNT advanced, firmware list restored byte-equal${hw_note})"
    exit 0
fi
r=""
for x in "${other[@]}"; do r="$r${r:+; }$x"; done
say "verdicts: FAIL ($r)"
exit 3
