#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_dlmap/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_dlmap image (src/main.c), with the
# `HH:MM:SS ` prefix stripped, and prints where a second display list may sit
# beside the firmware's (docs/notes/rpi5-display.md section 4, Q4):
#
#   rpi5_dlmap verdict: mem_words <n>   (spans <n>, zero spans <n>, END-bit words <n>)
#   rpi5_dlmap verdict: ch<n> heade <w>: <k> entries, stop <END at word W | NEXT 0 at word W | ...>
#   rpi5_dlmap verdict: fwend: word <W>|none
#       the word of the END that stopped channel 0's walk by CTL0.NEXT
#   rpi5_dlmap verdict: q4place: after-end <start>|before-head <start>|none (<reason>)
#       after-end: the image's first zero span of at least 20 words after
#       channel 0's END (start at word 32 or later); before-head: the first
#       zero span of at least 20 words at word 32 or later that ends below
#       channel 0's HEADE, used when there is no such span after the END;
#       none: neither (the reason says which)
#   rpi5_dlmap verdict: end: <the image's end line, or missing>
#   rpi5_dlmap verdict: done | FAIL (<reason>) | incomplete (<reason>)
#
# The image's lines (all prefixed `rpi5_dlmap: `; others are ignored):
#
#   hvs version 0x.. stepping D0|C0|unknown cxm_size 0x..
#   mem_words <n> (CXM_SIZE <n>, mapped list words <n>)
#   ch<n> ctrl0 0x.. lptrs 0x.. heade <w> dl 0x.. status 0x..
#   span <start>..<end> n=<count>  |  span ... more <n>
#   zero <start>..<end>            |  zero ... more <n>
#   end-word <i> 0x<val>           |  end-word ... more <n>
#   walk ch<n> entry <k> word <i> ctl0 0x.. valid <0|1> next <n>
#   walk ch<n> stop END at word <W> | NEXT 0 at word <W> | past memory at word <W> | step bound at word <W>
#   after-end zero <start>..<end>  |  after-end none (<reason>)
#   done (...) | FAIL (<reason>)
#
# Exit status: 0 the image said done, 1 incomplete (no end line), 2 usage,
# 3 the image said FAIL, or said done without the lines the answer needs
# (mem_words, channel 0's line and walk).
#
# Self-test: tools/probes/rpi_5/rpi5_dlmap/verdicts_selftest.sh
set -u

if [ $# -ne 1 ]; then
    echo "usage: ${0##*/} <console.log>" >&2
    exit 2
fi
if [ ! -r "$1" ] || [ -d "$1" ]; then
    echo "${0##*/}: cannot read $1" >&2
    exit 2
fi

# Consoles carry CR line ends; strip them before awk sees the fields.
tr -d '\r' <"$1" | awk '
BEGIN {
    endline = ""
    fwend = -1
}
{
    sub(/^.*rpi5_dlmap: /, "rpi5_dlmap: ")
}
/^rpi5_dlmap: (done|FAIL) \(/ {
    endline = $0
    sub(/^rpi5_dlmap: /, "", endline)
    next
}
/^rpi5_dlmap: mem_words [0-9]+/ {
    mem = $3 + 0
    mem_seen = 1
    next
}
/^rpi5_dlmap: ch[0-2] ctrl0 / {
    ch = substr($2, 3, 1) + 0
    ch_seen[ch] = 1
    for (i = 1; i < NF; i++)
        if ($i == "heade")
            heade[ch] = $(i + 1) + 0
    next
}
/^rpi5_dlmap: span [0-9]/ {
    nspan++
    next
}
/^rpi5_dlmap: span \.\.\. more / {
    nspan += $5
    next
}
/^rpi5_dlmap: zero [0-9]/ {
    split($3, a, "[.][.]")
    nzero++
    zs[nzero] = a[1] + 0
    ze[nzero] = a[2] + 0
    next
}
/^rpi5_dlmap: zero \.\.\. more / {
    zero_more = $5 + 0
    next
}
/^rpi5_dlmap: end-word [0-9]/ {
    nend++
    next
}
/^rpi5_dlmap: end-word \.\.\. more / {
    nend += $5
    next
}
/^rpi5_dlmap: walk ch[0-2] entry / {
    c = substr($3, 3, 1) + 0
    entries[c]++
    next
}
/^rpi5_dlmap: walk ch[0-2] stop / {
    c = substr($3, 3, 1) + 0
    stop[c] = $0
    sub(/^rpi5_dlmap: walk ch[0-2] stop /, "", stop[c])
    if ($5 == "END" && c == 0)
        fwend = $8 + 0
    next
}
/^rpi5_dlmap: after-end zero [0-9]/ {
    split($4, a, "[.][.]")
    ae_start = a[1] + 0
    ae_seen = 1
    next
}
/^rpi5_dlmap: after-end none / {
    ae_none = $0
    sub(/^rpi5_dlmap: after-end none /, "", ae_none)
    next
}
END {
    zm = (zero_more > 0) ? (" + " zero_more " more") : ""
    if (mem_seen)
        printf "rpi5_dlmap verdict: mem_words %d (spans %d, zero spans %d%s, END-bit words %d)\n", mem, nspan, nzero, zm, nend
    else
        print "rpi5_dlmap verdict: mem_words missing"
    for (c = 0; c <= 2; c++) {
        if (!(c in ch_seen))
            continue
        printf "rpi5_dlmap verdict: ch%d heade %d: %d entries, stop %s\n", c, heade[c], entries[c] + 0, (c in stop) ? stop[c] : "missing"
    }

    # ---- fwend ----
    if (fwend >= 0)
        printf "rpi5_dlmap verdict: fwend: word %d\n", fwend
    else
        print "rpi5_dlmap verdict: fwend: none"

    # ---- q4place ----
    q4 = ""
    if (fwend >= 0 && ae_seen)
        q4 = "after-end " ae_start
    else {
        # a zero span of at least 20 words, from word 32 on, below channel 0 HEADE
        for (i = 1; i <= nzero && q4 == ""; i++) {
            s = zs[i] < 32 ? 32 : zs[i]
            if ((0 in ch_seen) && ze[i] < heade[0] && ze[i] - s + 1 >= 20)
                q4 = "before-head " s
        }
        if (q4 == "") {
            if (!(0 in ch_seen))
                q4 = "none (no channel 0 line in the log)"
            else if (fwend >= 0)
                q4 = "none (no zero span of 20 words after the END at word " fwend ", none below HEADE " heade[0] ")"
            else
                q4 = "none (channel 0 walk found no END, no zero span of 20 words from word 32 below HEADE " heade[0] ")"
        }
    }
    printf "rpi5_dlmap verdict: q4place: %s\n", q4

    # ---- end ----
    if (endline != "")
        printf "rpi5_dlmap verdict: end: %s\n", endline
    else
        print "rpi5_dlmap verdict: end: missing (the log stops before the image'"'"'s end line)"
    if (endline ~ /^done \(/ && (!mem_seen || !(0 in ch_seen) || !(0 in stop))) {
        printf "rpi5_dlmap verdict: FAIL (the image said done but the log lacks %s)\n", !mem_seen ? "mem_words" : (!(0 in ch_seen) ? "the channel 0 line" : "the channel 0 walk")
        exit 3
    }
    if (endline ~ /^done \(/) {
        print "rpi5_dlmap verdict: done"
        exit 0
    }
    if (endline ~ /^FAIL \(/) {
        r = endline
        sub(/^FAIL \(/, "", r)
        sub(/\)$/, "", r)
        printf "rpi5_dlmap verdict: FAIL (%s)\n", r
        exit 3
    }
    print "rpi5_dlmap verdict: incomplete (no end line)"
    exit 1
}'
