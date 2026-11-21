#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_mbox/verdicts_selftest.sh
#
# Feeds verdicts.sh canned rpi5_mbox console logs, written in the image's line
# format (src/main.c), and checks every line it prints and its exit status:
#
#   all          every request answered in every way (exit 0); also checks the
#                `cache: on` line from the image's `cache management on` line
#                (`cut-early` checks `cache: missing ...`)
#   cacheoff     the same log with `cache management off` (exit 0, `cache: off`)
#   eio          2026-11-19's rpi5_display session as this image would see it:
#                every way -EIO with response code 0 (exit 1, q1 "no way")
#   cache        the display way -EIO, the answer in memory, clear and board
#                answered (exit 0, q1 "cache maintenance")
#   missing      one request's lines (clock disp) left out (exit 0)
#   truncated    the log stops inside request 3, no end line (exit 0: the
#                GET_THROTTLED lines are there)
#   cut-early    the log stops before any request (exit 1, every verdict line
#                still printed)
#   crlf         the "all" log with CR line ends and a timestamp prefix
#   fbok, fbno, fbcut, fbfail   the framebuffer GET tags answered, not answered,
#                cut before the geometry line, and beside a failing run; the
#                logs above have none (`fb: missing`); information only
#   usage        no argument, an unreadable file (exit 2)
#
# Prints one line per failed check and a last line
# "verdicts_selftest: N checks, M failed"; exits 0 when none failed.
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_mbox_verdicts.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM

checks=0
failed=0

fail() {
    failed=$((failed + 1))
    echo "FAIL: $*"
}

IDS="0x00030046 0x00000001 0x00030002:4 0x00030002:16 0x00030002:13 0x00030002:14 0x00030002:5"

value_of() {
    case "$1" in
    0x00030046) echo 0x00000000 ;;
    0x00000001) echo 0x6735f0a2 ;;
    0x00030002:5) echo 0x1dcd6500 ;; # 500000000
    *) echo 0x2faf0800 ;;             # 800000000
    esac
}

# request_lines <id> <display ok|eio> <memory ok|eio> <clear ok|eio> <board ok|eio>
request_lines() {
    id=$1
    v=$(value_of "$id")
    case "$id" in *:*) len=0x80000008 ;; *) len=0x80000004 ;; esac
    echo "rpi5_mbox: regs tag $id way display: before sta0 0x40000000 cnf0 0x00000000 sta1 0x40000000, after sta0 0x40000000 cnf0 0x00000000 sta1 0x40000000, sent 0x00213048 reply 0x00213048, 0 other message(s) dropped (last 0xffffffff), replied"
    if [ "$2" = ok ]; then
        echo "rpi5_mbox: tag $id status 0x80000000 answered $len value $v rc 0 way display (no clean before, no invalidate after: rpi5_display's path; buffer PA 0x213040)"
    else
        echo "rpi5_mbox: tag $id status 0x00000000 answered 0x00000000 value 0x00000000 rc -5 way display (no clean before, no invalidate after: rpi5_display's path; buffer PA 0x213040)"
    fi
    if [ "$3" = ok ]; then
        echo "rpi5_mbox: memory tag $id status 0x80000000 answered $len value $v way display (the same buffer read uncached after the wait: size 0x0000001c tag 0x00030046; before the doorbell size 0x00000000 code 0x00000000 tag 0x00000000)"
    else
        echo "rpi5_mbox: memory tag $id status 0x00000000 answered 0x00000000 value 0x00000000 way display (the same buffer read uncached after the wait: size 0x00000000 tag 0x00000000; before the doorbell size 0x00000000 code 0x00000000 tag 0x00000000)"
    fi
    echo "rpi5_mbox: regs tag $id way clear: before sta0 0x40000000 cnf0 0x00000000 sta1 0x40000000, after sta0 0x40000000 cnf0 0x00000000 sta1 0x40000000, sent 0x00213088 reply 0x00213088, 0 other message(s) dropped (last 0xffffffff), replied"
    if [ "$4" = ok ]; then
        echo "rpi5_mbox: tag $id status 0x80000000 answered $len value $v rc 0 way clear (clean before 0, invalidate after 0: rpi5_clear's path; buffer PA 0x213080)"
    else
        echo "rpi5_mbox: tag $id status 0x00000000 answered 0x00000000 value 0x00000000 rc -5 way clear (clean before 0, invalidate after 0: rpi5_clear's path; buffer PA 0x213080)"
    fi
    if [ "$5" = ok ]; then
        echo "rpi5_mbox: tag $id status n/a answered $len value $v rc 0 way board (rpi_mbox_request_timed: write 10 poll 900 inval 5 cycles)"
    else
        echo "rpi5_mbox: tag $id status n/a answered 0x00000000 value 0x00000000 rc -5 way board (rpi_mbox_request_timed: write 10 poll 900 inval 5 cycles)"
    fi
}

head_lines() {
    echo "*** Booting Zephyr OS build v4.3.0 ***"
    echo "board: PM watchdog resets the SoC ~70 s after boot (tryboot -> Linux)"
    echo "rpi5_mbox: build 0123abcd"
    echo "rpi5_mbox: cache management on"
    echo "rpi5_mbox: cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1, data line 64 bytes"
    echo "rpi5_mbox: mailbox init 0 (rpi_mbox_init)"
    echo "rpi5_mbox: regs at start: sta0 0x40000000 cnf0 0x00000000 sta1 0x40000000"
}

# log <display> <memory> <clear> <board> [skip-id]
log() {
    head_lines
    i=1
    for id in $IDS; do
        if [ "$id" != "${5:-}" ]; then
            echo "rpi5_mbox: request $i of 7: x, tag $id"
            request_lines "$id" "$1" "$2" "$3" "$4"
        fi
        i=$((i + 1))
    done
    echo "rpi5_mbox: regs at end: sta0 0x40000000 cnf0 0x00000000 sta1 0x40000000"
}

# run <name> <expected exit> <log file>: output in $tmp/<name>.out
run() {
    checks=$((checks + 1))
    sh "$V" "$3" >"$tmp/$1.out" 2>"$tmp/$1.err"
    rc=$?
    if [ "$rc" -ne "$2" ]; then
        fail "$1: exit $rc, expected $2"
        sed 's/^/    /' "$tmp/$1.out" "$tmp/$1.err"
    fi
}

# has <name> <exact line>
has() {
    checks=$((checks + 1))
    grep -Fxq -- "$2" "$tmp/$1.out" || {
        fail "$1: no line: $2"
        sed 's/^/    got: /' "$tmp/$1.out"
    }
}

# fb_lines <ok|no>: the five framebuffer GET tags (clear way) and the geometry line
fb_lines() {
    if [ "$1" = ok ]; then
        echo "rpi5_mbox: tag 0x00040003 status 0x80000000 answered 0x80000008 value 0x00000780 value2 0x00000438 rc 0 way clear (clean before 0, invalidate after 0: get_physical_width_height, a GET tag only)"
        echo "rpi5_mbox: tag 0x00040004 status 0x80000000 answered 0x80000008 value 0x00000780 value2 0x00000438 rc 0 way clear (clean before 0, invalidate after 0: get_virtual_width_height, a GET tag only)"
        echo "rpi5_mbox: tag 0x00040005 status 0x80000000 answered 0x80000004 value 0x00000010 rc 0 way clear (clean before 0, invalidate after 0: get_depth, a GET tag only)"
        echo "rpi5_mbox: tag 0x00040006 status 0x80000000 answered 0x80000004 value 0x00000001 rc 0 way clear (clean before 0, invalidate after 0: get_pixel_order, a GET tag only)"
        echo "rpi5_mbox: tag 0x00040008 status 0x80000000 answered 0x80000004 value 0x00000f00 rc 0 way clear (clean before 0, invalidate after 0: get_pitch, a GET tag only)"
        echo "rpi5_mbox: fb geometry 1920x1080 depth 16 pitch 3840 order rgb (virtual 1920x1080)"
    else
        for t in 0x00040003 0x00040004 0x00040005 0x00040006 0x00040008; do
            echo "rpi5_mbox: tag $t status 0x00000000 answered 0x00000000 value 0x00000000 rc -5 way clear (clean before 0, invalidate after 0: x, a GET tag only)"
        done
        echo "rpi5_mbox: fb geometry not answered (physical no, virtual no, depth no, order no, pitch no)"
    fi
}

# lines <name> <count>: every full run prints 1 (cache) + 7 + 4 lines (q1, fb, end, verdict)
lines() {
    checks=$((checks + 1))
    got=$(wc -l <"$tmp/$1.out" | tr -d ' ')
    [ "$got" -eq "$2" ] || fail "$1: $got lines, expected $2"
}

ALLW="display yes, memory yes, clear yes, board yes"

# ---- all answered
{ log ok ok ok ok; echo "rpi5_mbox: answered: display 7 of 7, memory 7 of 7, clear 7 of 7, board 7 of 7"; echo "rpi5_mbox: done (7 of 7 answered)"; echo "rpi5_mbox: feeding stopped, reset expected"; } >"$tmp/all.log"
run all 0 "$tmp/all.log"
lines all 12
has all "rpi5_mbox verdict: cache: on"
has all "rpi5_mbox verdict: 0x00030046 get_throttled: answered ($ALLW) value 0x00000000"
has all "rpi5_mbox verdict: 0x00000001 get_firmware_revision: answered ($ALLW) value 0x6735f0a2"
has all "rpi5_mbox verdict: 0x00030002:4 clock_core: answered ($ALLW) value 0x2faf0800 (800000000 Hz)"
has all "rpi5_mbox verdict: 0x00030002:5 clock_v3d: answered ($ALLW) value 0x1dcd6500 (500000000 Hz)"
has all "rpi5_mbox verdict: q1: not reproduced (the display way answered 7 of 7; clear 7, board 7)"
has all "rpi5_mbox verdict: end: done (7 of 7 answered)"
has all "rpi5_mbox verdict: PASS (GET_THROTTLED answered by display clear board)"

# ---- framebuffer tags: absent (the logs above), answered, not answered
has all "rpi5_mbox verdict: fb: missing (no framebuffer tag line in the log)"
{ log ok ok ok ok; fb_lines ok; echo "rpi5_mbox: done (7 of 7 answered)"; } >"$tmp/fbok.log"
run fbok 0 "$tmp/fbok.log"
lines fbok 12
has fbok "rpi5_mbox verdict: fb: geometry 1920x1080 depth 16 pitch 3840 order rgb (virtual 1920x1080) (tags answered 5 of 5)"
has fbok "rpi5_mbox verdict: PASS (GET_THROTTLED answered by display clear board)"
has fbok "rpi5_mbox verdict: 0x00030002:4 clock_core: answered ($ALLW) value 0x2faf0800 (800000000 Hz)"
{ log ok ok ok ok; fb_lines no; echo "rpi5_mbox: done (7 of 7 answered)"; } >"$tmp/fbno.log"
run fbno 0 "$tmp/fbno.log"
lines fbno 12
has fbno "rpi5_mbox verdict: fb: geometry not answered (physical no, virtual no, depth no, order no, pitch no) (tags answered 0 of 5)"
has fbno "rpi5_mbox verdict: PASS (GET_THROTTLED answered by display clear board)"
# tag lines but no geometry line (cut inside the fb section); a failing run keeps its FAIL
{ log ok ok ok ok; fb_lines ok | head -2; } >"$tmp/fbcut.log"
run fbcut 0 "$tmp/fbcut.log"
has fbcut "rpi5_mbox verdict: fb: no geometry line (tags answered 2 of 5, 2 line(s) seen)"
{ log eio eio eio eio; fb_lines ok; echo "rpi5_mbox: FAIL (GET_THROTTLED not answered in any way; 0 of 7 answered)"; } >"$tmp/fbfail.log"
run fbfail 1 "$tmp/fbfail.log"
has fbfail "rpi5_mbox verdict: FAIL (GET_THROTTLED not answered)"
has fbfail "rpi5_mbox verdict: 0x00030046 get_throttled: not answered (display no, memory no, clear no, board no)"

# ---- every way -EIO with code 0 (2026-11-19)
{ log eio eio eio eio; echo "rpi5_mbox: FAIL (GET_THROTTLED not answered in any way; 0 of 7 answered)"; } >"$tmp/eio.log"
run eio 1 "$tmp/eio.log"
lines eio 12
has eio "rpi5_mbox verdict: cache: on"
has eio "rpi5_mbox verdict: 0x00030046 get_throttled: not answered (display no, memory no, clear no, board no)"
has eio "rpi5_mbox verdict: 0x00030002:13 clock_hdmi-hsm: not answered (display no, memory no, clear no, board no)"
has eio "rpi5_mbox verdict: q1: no way answered (display, clear and board 0 of 7: transport, base or firmware; see the regs lines)"
has eio "rpi5_mbox verdict: end: FAIL (GET_THROTTLED not answered in any way; 0 of 7 answered)"
has eio "rpi5_mbox verdict: FAIL (GET_THROTTLED not answered)"

# ---- the cache explanation
{ log eio ok ok ok; echo "rpi5_mbox: done (7 of 7 answered)"; } >"$tmp/cache.log"
run cache 0 "$tmp/cache.log"
has cache "rpi5_mbox verdict: cache: on"
has cache "rpi5_mbox verdict: 0x00030046 get_throttled: answered (display no, memory yes, clear yes, board yes) value 0x00000000"
has cache "rpi5_mbox verdict: q1: cache maintenance (display 0 of 7, memory 7, clear 7, board 7: without the clean and the invalidate the CPU does not see the answer)"
has cache "rpi5_mbox verdict: PASS (GET_THROTTLED answered by clear board)"

# ---- display -EIO, memory answered, clear and board -EIO
{ log eio ok eio eio; echo "rpi5_mbox: FAIL (GET_THROTTLED not answered in any way; 0 of 7 answered)"; } >"$tmp/memonly.log"
run memonly 1 "$tmp/memonly.log"
has memonly "rpi5_mbox verdict: cache: on"
has memonly "rpi5_mbox verdict: 0x00030046 get_throttled: not answered (display no, memory yes, clear no, board no) (the firmware answer was in memory: value 0x00000000)"
has memonly "rpi5_mbox verdict: q1: no way answered (memory 7 of 7: the firmware wrote answers no CPU read showed; see the regs lines)"

# ---- one request missing
{ log ok ok ok ok 0x00030002:16; echo "rpi5_mbox: done (6 of 7 answered)"; } >"$tmp/missing.log"
run missing 0 "$tmp/missing.log"
lines missing 12
has missing "rpi5_mbox verdict: cache: on"
has missing "rpi5_mbox verdict: 0x00030002:16 clock_disp: missing (display -, memory -, clear -, board -) (no line of this request in the log)"
has missing "rpi5_mbox verdict: 0x00030002:13 clock_hdmi-hsm: answered ($ALLW) value 0x2faf0800 (800000000 Hz)"
has missing "rpi5_mbox verdict: PASS (GET_THROTTLED answered by display clear board)"

# ---- truncated inside request 3 (after its display lines)
log ok ok ok ok | awk '/request 3 of 7/ { n = NR } n && NR > n + 3 { exit } { print }' >"$tmp/truncated.log"
run truncated 0 "$tmp/truncated.log"
lines truncated 12
has truncated "rpi5_mbox verdict: cache: on"
has truncated "rpi5_mbox verdict: 0x00030002:4 clock_core: answered (display yes, memory yes, clear -, board -) value 0x2faf0800 (800000000 Hz)"
has truncated "rpi5_mbox verdict: 0x00030002:5 clock_v3d: missing (display -, memory -, clear -, board -) (no line of this request in the log)"
has truncated "rpi5_mbox verdict: end: missing (the log stops before the image's end line)"
has truncated "rpi5_mbox verdict: PASS (GET_THROTTLED answered by display clear board)"

# ---- cut before any request
head_lines | head -3 >"$tmp/early.log"
run early 1 "$tmp/early.log"
lines early 12
has early "rpi5_mbox verdict: cache: missing (no cache management line in the log)"
has early "rpi5_mbox verdict: 0x00030046 get_throttled: missing (display -, memory -, clear -, board -) (no line of this request in the log)"
has early "rpi5_mbox verdict: q1: unknown (no request line in the log)"
has early "rpi5_mbox verdict: end: missing (the log stops before the image's end line)"
has early "rpi5_mbox verdict: FAIL (GET_THROTTLED: no line in the log)"

# ---- CONFIG_CACHE_MANAGEMENT off (the way rpi5_display built)
sed 's/^rpi5_mbox: cache management on$/rpi5_mbox: cache management off/' "$tmp/all.log" >"$tmp/cacheoff.log"
run cacheoff 0 "$tmp/cacheoff.log"
has cacheoff "rpi5_mbox verdict: cache: off"

# ---- CR line ends and a console timestamp prefix
sed 's/^/[00:00:01.000,000] /; s/$/\r/' "$tmp/all.log" >"$tmp/crlf.log"
run crlf 0 "$tmp/crlf.log"
has crlf "rpi5_mbox verdict: cache: on"
has crlf "rpi5_mbox verdict: 0x00030002:14 clock_hdmi-bvb: answered ($ALLW) value 0x2faf0800 (800000000 Hz)"
has crlf "rpi5_mbox verdict: PASS (GET_THROTTLED answered by display clear board)"

# ---- usage
checks=$((checks + 1))
sh "$V" >/dev/null 2>&1
[ $? -eq 2 ] || fail "usage: no argument did not exit 2"
checks=$((checks + 1))
sh "$V" "$tmp/does-not-exist" >/dev/null 2>&1
[ $? -eq 2 ] || fail "usage: an unreadable file did not exit 2"

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
