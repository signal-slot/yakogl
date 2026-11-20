#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_mbox/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_mbox image (src/main.c) and prints, always
# in this order and always all of them:
#
#   rpi5_mbox verdict: <id> <name>: <state> (display <s>, memory <s>, clear <s>, board <s>)[ value ...]
#                         one line per request, seven, in the image's order
#   rpi5_mbox verdict: q1: <cause> (...)
#   rpi5_mbox verdict: end: <the image's end line, or missing>
#   rpi5_mbox verdict: PASS (GET_THROTTLED answered by <ways>) | FAIL (<reason>)
#
# The image's request lines (other lines are ignored):
#
#   rpi5_mbox: tag <id> status <hex|n/a> answered <hex> value <hex> rc <n> way <display|clear|board> ...
#   rpi5_mbox: memory tag <id> status <hex> answered <hex> value <hex> way display ...
#
# A way is answered (`yes`) when its rc is 0 (not for `memory`, which has
# none), its status is 0x80000000 (`n/a` for board, which keeps that word) and
# bit 31 of its answered word is set; `no` when its line is there and is not;
# `-` when its line is missing. The request's state is `answered` when the
# display, clear or board way said yes, `not answered` when at least one of
# their lines is there and none said yes, `missing` when the log has no line
# of the request at all. `memory` is evidence, not an answer the CPU saw: an
# answer in memory only means the CPU's cached copy hid it. The value is the
# answering way's (clear, then board, then display), with the rate in Hz for a
# clock. A log holding two boots counts the later line of each.
#
# q1 (docs/notes/rpi5-display.md section 4, question 1), from the counts of
# the lines seen:
#   cache maintenance   no display way answered and a clear or board way did
#   not reproduced      a display way answered
#   no way answered     lines of display, clear or board seen, none answered
#   unknown             no request line in the log
#
# PASS rule (section 3): GET_THROTTLED (0x00030046) answered in some way.
#
# Exit status: 0 PASS, 1 FAIL (including a log without the GET_THROTTLED lines
# or without an end line still says what it saw), 2 usage.
#
# Self-test: tools/probes/rpi_5/rpi5_mbox/verdicts_selftest.sh
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
function hexval(s,    i, c, v, d) {
    v = 0
    s = tolower(s)
    if (substr(s, 1, 2) != "0x")
        return -1
    for (i = 3; i <= length(s); i++) {
        c = substr(s, i, 1)
        d = index("0123456789abcdef", c)
        if (d == 0)
            return -1
        v = v * 16 + d - 1
    }
    return v
}
# bit 31 of a word given as 0x........
function bit31(s,    v) {
    v = hexval(s)
    return v >= 2147483648
}
# the value after the key word `k` on the current line, or ""
function field(k,    i) {
    for (i = 1; i < NF; i++)
        if ($i == k)
            return $(i + 1)
    return ""
}
BEGIN {
    n = split("0x00030046 0x00000001 0x00030002:4 0x00030002:16 0x00030002:13 0x00030002:14 0x00030002:5", ids, " ")
    split("get_throttled get_firmware_revision clock_core clock_disp clock_hdmi-hsm clock_hdmi-bvb clock_v3d", names, " ")
    for (i = 1; i <= n; i++)
        known[ids[i]] = i
    nways = split("display memory clear board", ways, " ")
    endline = ""
}
{
    sub(/^.*rpi5_mbox: /, "rpi5_mbox: ")
}
/^rpi5_mbox: (done|FAIL) \(/ {
    endline = $0
    sub(/^rpi5_mbox: /, "", endline)
    next
}
/^rpi5_mbox: (memory )?tag / {
    mem = ($2 == "memory")
    id = mem ? $4 : $3
    if (!(id in known))
        next
    way = field("way")
    if (way != "display" && way != "clear" && way != "board")
        next
    if (mem && way != "display")
        next
    if (mem)
        way = "memory" # the buffer of the display way, read uncached
    st = field("status")
    an = field("answered")
    va = field("value")
    rc = field("rc")
    ok = 1
    if (!mem && rc != "0")
        ok = 0
    if (way == "board") {
        if (st != "n/a")
            ok = 0
    } else if (tolower(st) != "0x80000000") {
        ok = 0
    }
    if (hexval(an) < 0 || !bit31(an))
        ok = 0
    if (hexval(va) < 0)
        ok = 0
    seen[id, way] = 1
    yes[id, way] = ok
    val[id, way] = va
}
END {
    nd = nm = nc = nb = 0
    sd = sc = sb = 0
    for (i = 1; i <= n; i++) {
        id = ids[i]
        any = 0
        cpu = 0
        detail = ""
        for (w = 1; w <= nways; w++) {
            k = ways[w]
            if (!((id, k) in seen))
                s = "-"
            else {
                any = 1
                s = yes[id, k] ? "yes" : "no"
            }
            detail = detail (w > 1 ? ", " : "") k " " s
        }
        if (yes[id, "display"]) nd++
        if (yes[id, "memory"]) nm++
        if (yes[id, "clear"]) nc++
        if (yes[id, "board"]) nb++
        if ((id, "display") in seen) sd++
        if ((id, "clear") in seen) sc++
        if ((id, "board") in seen) sb++
        by = ""
        if (yes[id, "display"]) by = by " display"
        if (yes[id, "clear"]) by = by " clear"
        if (yes[id, "board"]) by = by " board"
        if (by != "") {
            state = "answered"
            v = yes[id, "clear"] ? val[id, "clear"] : (yes[id, "board"] ? val[id, "board"] : val[id, "display"])
            extra = " value " v
            if (index(id, ":") > 0)
                extra = extra " (" hexval(v) " Hz)"
        } else if ((id, "display") in seen || (id, "clear") in seen || (id, "board") in seen) {
            state = "not answered"
            extra = yes[id, "memory"] ? " (the firmware answer was in memory: value " val[id, "memory"] ")" : ""
        } else {
            state = "missing"
            extra = " (no line of this request in the log)"
        }
        answeredby[id] = by
        printf "rpi5_mbox verdict: %s %s: %s (%s)%s\n", id, names[i], state, detail, extra
    }
    if (sd + sc + sb == 0)
        q = "unknown (no request line in the log)"
    else if (nd > 0)
        q = "not reproduced (the display way answered " nd " of " n "; clear " nc ", board " nb ")"
    else if (nc + nb > 0)
        q = "cache maintenance (display 0 of " n ", memory " nm ", clear " nc ", board " nb ": without the clean and the invalidate the CPU does not see the answer)"
    else if (nm > 0)
        q = "no way answered (memory " nm " of " n ": the firmware wrote answers no CPU read showed; see the regs lines)"
    else
        q = "no way answered (display, clear and board 0 of " n ": transport, base or firmware; see the regs lines)"
    printf "rpi5_mbox verdict: q1: %s\n", q
    if (endline != "")
        printf "rpi5_mbox verdict: end: %s\n", endline
    else
        printf "rpi5_mbox verdict: end: missing (the log stops before the image'"'"'s end line)\n"
    t = ids[1]
    if (answeredby[t] != "") {
        printf "rpi5_mbox verdict: PASS (GET_THROTTLED answered by%s)\n", answeredby[t]
        exit 0
    }
    if ((t, "display") in seen || (t, "clear") in seen || (t, "board") in seen)
        printf "rpi5_mbox verdict: FAIL (GET_THROTTLED not answered)\n"
    else
        printf "rpi5_mbox verdict: FAIL (GET_THROTTLED: no line in the log)\n"
    exit 1
}'
