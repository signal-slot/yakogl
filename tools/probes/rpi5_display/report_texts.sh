#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   report_texts.sh --verdicts <console.log> [--linux-ref <file>]
#                   [--throttled <file|value>]... --record <name> --commit <sha>
#
# Prints the report texts of the U3 board session (Pi 5 display plan) as one
# Markdown block: the verdict lines (c5), the comment for issue #7, the U3 row
# for driver/docs/rpi5-v3d71-facts.md section 13, the P11 row for
# docs/notes/pre-study-rpi5.md section 8 (both for #321's owner, this script
# edits neither) and a title and text for each open item (c6). The verdict
# lines come from verdicts.sh in this directory. A line the log lacks reads
# '(missing)'.
#
# --record is the record directory name only (YYYYMMDD-HHMM-<name>-<sha8>).
# --throttled takes a file or a value (0x50005, 0), once per record.
#
# Redaction: if any input (the files' contents and the argument values) looks
# like an IPv4 address, a /dev/tty or /dev/serial path, a tty device name, a
# host name with a dot, or a USB serial, nothing is printed but the reason
# (class only, never the value) and the exit status is 2.
#
# Exit: 0 printed, 2 usage error or refused input.
# Self-test: tools/probes/rpi5_display/report_texts_selftest.sh
set -u

PROG=${0##*/}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

usage() {
    echo "usage: $PROG --verdicts <log> [--linux-ref <file>] [--throttled <file|value>]... --record <name> --commit <sha>" >&2
    exit 2
}

VERD="" REF="" REC="" COMMIT=""
THR=()
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || usage
    case $1 in
        --verdicts) VERD=$2 ;;
        --linux-ref) REF=$2 ;;
        --throttled) THR+=("$2") ;;
        --record) REC=$2 ;;
        --commit) COMMIT=$2 ;;
        *) usage ;;
    esac
    shift 2
done
[ -n "$VERD" ] && [ -n "$REC" ] && [ -n "$COMMIT" ] || usage

# class of the first look-alike in stdin ("" when none); never prints a value
redact_class() {
    local t
    t=$(cat)
    if grep -Eq '(^|[^[:digit:].])[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}([^[:digit:]]|$)' <<< "$t"; then echo "an IPv4 address"
    elif grep -Eq '/dev/(tty|serial)' <<< "$t"; then echo "a /dev/tty or /dev/serial path"
    elif grep -Eq '(^|[^[:alnum:]_-])tty(S|USB|ACM|AMA)[0-9]+' <<< "$t"; then echo "a tty device name"
    elif grep -Eiq '(^|[^[:alnum:]_.-])[a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*\.(local|lan|home|internal|localdomain|com|net|org|io|jp|dev|co|de|uk|us)([^[:alnum:]_-]|$)' <<< "$t"; then echo "a host name with a dot"
    elif grep -Eiq 'usb-[a-z0-9_.-]+-if[0-9]|(serial( ?number)?|iSerial)[ :=]+[a-z0-9]{6,}' <<< "$t"; then echo "a USB serial"
    fi
}

refuse() { echo "$PROG: refused: $1 looks like $2 (records must be redacted); nothing printed" >&2; exit 2; }
check_text() { local c; c=$(printf '%s\n' "$2" | redact_class); [ -z "$c" ] || refuse "$1" "$c"; }
check_file() { local c; c=$(redact_class < "$2"); [ -z "$c" ] || refuse "$1" "$c"; }

[ -r "$VERD" ] || { echo "$PROG: cannot read the --verdicts file" >&2; exit 2; }
[ -z "$REF" ] || [ -r "$REF" ] || { echo "$PROG: cannot read the --linux-ref file" >&2; exit 2; }
check_text "--record" "$REC"
check_text "--commit" "$COMMIT"
check_file "--verdicts" "$VERD"
[ -z "$REF" ] || check_file "--linux-ref" "$REF"
for t in "${THR[@]+"${THR[@]}"}"; do
    if [ -f "$t" ]; then check_file "--throttled" "$t"; else check_text "--throttled" "$t"; fi
done

grep -Eq '^[0-9]{8}-[0-9]{4}-[A-Za-z0-9_]+(-[A-Za-z0-9_]+)*-[0-9a-f]{8}$' <<< "$REC" || {
    echo "$PROG: --record is not of the form YYYYMMDD-HHMM-<name>-<sha8>" >&2; exit 2; }
grep -Eq '^[0-9a-f]{7,40}$' <<< "$COMMIT" || { echo "$PROG: --commit is not a hex sha" >&2; exit 2; }

# ---- verdict lines (verdicts.sh output; already passed through its redactor)
if [ -n "$REF" ]; then VOUT=$("$HERE/verdicts.sh" "$VERD" "$REF" 2>&1); else VOUT=$("$HERE/verdicts.sh" "$VERD" 2>&1); fi
vrc=$?
[ "$vrc" -le 1 ] || { echo "$PROG: verdicts.sh failed" >&2; exit 2; }

NAMES=("u3 scanout" "u3 plane" "u3 adopt-readback" "u3 mailbox-fb" "u3 mailbox-readback" "u3 scanout-follows-mailbox" "u3 split-screen" "rpi5_display")
declare -A L
for i in 1 2 3 4 5 6 7 8; do
    L[$i]=$(sed -nE "s/^  \[$i\] //p" <<< "$VOUT" | head -n 1)
    [ -n "${L[$i]}" ] || L[$i]="(missing)"
done
ROUTELINE=$(grep -E '^verdicts: (route [ABC]|incomplete)' <<< "$VOUT" | head -n 1)
ROUTE=$(sed -nE 's/^verdicts: (route [ABC]|incomplete).*/\1/p' <<< "$ROUTELINE")
[ -n "$ROUTE" ] || ROUTE="(missing)"
REFLINES=$(grep -E '^(verdicts: linux_ref|  ref \[)' <<< "$VOUT")
MISSING=()
for i in 1 2 3 4 5 6 7 8; do
    [ "${L[$i]}" = "(missing)" ] && MISSING+=("${NAMES[$((i - 1))]}")
done

# ---- get_throttled
decode_thr() { # <value> -> short text
    local n=$(($1)) s=""
    if [ "$n" -eq 0 ]; then echo "clean"; return; fi
    ((n & 1)) && s="$s under-voltage-now"
    ((n & 2)) && s="$s freq-capped-now"
    ((n & 4)) && s="$s throttled-now"
    ((n & 8)) && s="$s soft-temp-limit-now"
    ((n & 0x10000)) && s="$s under-voltage-occurred"
    ((n & 0x20000)) && s="$s freq-cap-occurred"
    ((n & 0x40000)) && s="$s throttled-occurred"
    ((n & 0x80000)) && s="$s soft-temp-limit-occurred"
    echo "${s# }"
}
THRTXT=()
THRBAD=0
for t in "${THR[@]+"${THR[@]}"}"; do
    raw=$t
    [ -f "$t" ] && raw=$(grep -oE '0[xX][0-9a-fA-F]+|[0-9]+' "$t" | head -n 1)
    if grep -Eq '^(0[xX][0-9a-fA-F]+|[0-9]+)$' <<< "$raw"; then
        d=$(decode_thr "$raw")
        THRTXT+=("$raw ($d)")
        [ "$d" = "clean" ] || THRBAD=1
    else
        THRTXT+=("(missing)")
    fi
done
[ ${#THRTXT[@]} -gt 0 ] || THRTXT=("(missing)")
THRJOIN=$(printf '%s; ' "${THRTXT[@]}")
THRJOIN=${THRJOIN%; }

# ---- the sentence per route
case $ROUTE in
    "route A")
        RESULT="route A: the firmware's scanout runs under a kernel= image and its read-back matches; the display is adopted, not reprogrammed"
        ROW_STATUS="**Answered.**"
        ROWTXT="a kernel= image finds the firmware scanout running and the pattern read-back through it matches" ;;
    "route B")
        RESULT="route B: the BCM2712 property mailbox allocates a framebuffer and the read-back matches"
        ROW_STATUS="**Answered.**"
        ROWTXT="the property mailbox still hands out a framebuffer and the pattern read-back matches" ;;
    "route C")
        RESULT="route C: neither the firmware's scanout nor the mailbox framebuffer works; HVS6, pixel valve and HDMI have to be programmed by the image"
        ROW_STATUS="**Answered.**"
        ROWTXT="neither a running firmware scanout nor a mailbox framebuffer passes, so the image programs HVS6, pixel valve and HDMI" ;;
    *)
        RESULT="no route: the run is incomplete (missing: ${MISSING[*]:-(none)})"
        ROW_STATUS="**Answered in part.**"
        ROWTXT="the probe run stopped before all eight verdict lines" ;;
esac

# ---- output
echo "## U3 report texts"
echo
echo "### c5: verdict lines, record, get_throttled"
echo
echo "Record: \`records/rpi5/$REC/\` at commit \`$COMMIT\`"
echo
echo "get_throttled: $THRJOIN"
echo
echo '```'
for i in 1 2 3 4 5 6 7 8; do
    if [ "${L[$i]}" = "(missing)" ]; then echo "[$i] ${NAMES[$((i - 1))]}: (missing)"; else echo "[$i] ${L[$i]}"; fi
done
echo "${ROUTELINE:-verdicts: (missing)}"
[ -z "$REFLINES" ] || echo "$REFLINES"
echo '```'
echo
echo "### c6: comment for issue #7"
echo
echo "Pi 5 display (U3), commit \`$COMMIT\`, record \`$REC\`: $RESULT."
echo
echo "- scanout: ${L[1]}"
echo "- plane: ${L[2]}"
echo "- adopt read-back: ${L[3]}"
echo "- mailbox framebuffer: ${L[4]}"
echo "- mailbox read-back: ${L[5]}"
echo "- scanout follows mailbox: ${L[6]}"
echo "- split screen: ${L[7]}"
echo "- probe end: ${L[8]}"
echo "- get_throttled: $THRJOIN"
echo
echo "### Row for driver/docs/rpi5-v3d71-facts.md section 13 (U3), for #321's owner"
echo
echo "| **U3** Display | $ROW_STATUS Path and addresses are in section 10. U3 board session (record \`$REC\`, commit \`$COMMIT\`): $ROWTXT. Scanout: ${L[1]}; plane: ${L[2]}; mailbox framebuffer: ${L[4]}. get_throttled: $THRJOIN. |"
echo
echo "### Row for docs/notes/pre-study-rpi5.md section 8 (P11), for #321's owner"
echo
if [ "$ROUTE" = "incomplete" ] || [ "$ROUTE" = "(missing)" ]; then
    echo "| P11 Display (U3) | M to L | P0 | U3 probe run incomplete (record \`$REC\`); a frame on HDMI still open |"
else
    echo "| P11 Display (U3) | M to L | P0 | U3 answered: $RESULT (record \`$REC\`); a frame on HDMI follows on that route |"
fi
echo
echo "### Open items"
n=0
item() { n=$((n + 1)); echo; echo "#### Item $n: $1"; echo; echo "$2"; }
if [ ${#MISSING[@]} -gt 0 ]; then
    item "Pi 5 display probe: complete the U3 verdict lines" "The run (record \`$REC\`, commit \`$COMMIT\`) lacks: ${MISSING[*]}. Rerun the rpi5_display probe so that all eight lines print; the image must come back to Linux by its own watchdog."
fi
case $ROUTE in
    "route A") item "Pi 5 display: first picture on the adopted firmware scanout" "Route A passed (record \`$REC\`). Draw the split screen (text on top, pattern below) into the adopted plane and check it on HDMI or by read-back." ;;
    "route B") item "Pi 5 display: first picture through the mailbox framebuffer" "Route B passed (record \`$REC\`). Add the BCM2712 tags to the Pi 5 board-layer screen and draw the split screen." ;;
    "route C") item "Pi 5 display: program HVS6, pixel valve and HDMI" "Route C (record \`$REC\`): neither the firmware scanout nor the mailbox works. Bring up the clocks, the HVS6 channel, the pixel valve and HDMI0 from the facts file section 10." ;;
esac
if [ "$THRBAD" -eq 1 ]; then
    item "Pi 5 supply: get_throttled reports a fault during the U3 session" "get_throttled was $THRJOIN (record \`$REC\`). Check the supply before trusting timing-sensitive results."
fi
if grep -q 'differ' <<< "$REFLINES"; then
    item "Pi 5 display: probe plane differs from the Linux reference" "The plane fields of the probe and of the Linux reference differ (see the linux_ref lines in c5; record \`$REC\`). Decide which one the first picture follows."
fi
[ "$n" -gt 0 ] || { echo; echo "(none)"; }
exit 0
