#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_dispregs/verdicts_selftest.sh
#
# Feeds verdicts.sh canned rpi5_dispregs console logs, written in the image's
# line format (src/main.c), and checks the q3:/q5:/q6: lines and the exit
# status of each:
#
#   read       pv0 runs at 960 against HVS FWIDTH 1920, the HSM clock on with
#              a measured rate, both HDMI ports read (exit 0; q3 two pixels
#              per clock yes; q6 reads hdmi0 hotplug 1)
#   skipped    the HSM clock off: the image prints `hdmi skipped (hsm clock
#              off)` (exit 0; q6 says skipped, q5 says hsm off)
#   onepx      valve and HVS both 1920 (q3 two pixels per clock no)
#   stopped    neither valve runs (q3 no pixel valve runs)
#   notanswered  every mailbox tag failed (q5 not answered, q6 displays not
#              answered), the end line still done (exit 0)
#   fail       the image's FAIL end line (exit 3)
#   incomplete the log stops before the end line (exit 1)
#   crlf       the "read" log with CR line ends and a timestamp prefix
#   counts     the YLINE trace (in every log above): 1080 values, 3 wraps
#              over 3 FRCNT steps (q4yline: counts)
#   stuck      YLINE 0 in every sample (q4yline: stuck, exit 0)
#   fewvals    YLINE takes 40 values (q4yline: unknown, exit 0)
#   missing    no trace lines with a done end line (FAIL, exit 3); with no
#              end line (incomplete, exit 1) or a FAIL end line (its reason)
#   disagree   the image's q4yline says counts, its summary gives stuck
#              (FAIL, exit 3)
#   yskip      the image skipped the trace (the HVS did not map): unknown
#   usage      no argument, an unreadable file (exit 2)
#
# Prints one line per failed check and a last line
# "verdicts_selftest: N checks, M failed"; exits 0 when none failed.
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_dispregs_verdicts.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM

checks=0
failed=0

fail() {
    failed=$((failed + 1))
    echo "FAIL: $*"
}

# expect_has <name> <output> <substring>: the output holds the substring.
expect_has() {
    checks=$((checks + 1))
    case "$2" in
    *"$3"*) ;;
    *) fail "$1: missing '$3'" ;;
    esac
}

# expect_rc <name> <got> <want>
expect_rc() {
    checks=$((checks + 1))
    [ "$2" = "$3" ] || fail "$1: exit $2, wanted $3"
}

# tag <name> <clock> <rc> <value>: one clock tag line (rc 0: answered).
tag() {
    if [ "$3" = 0 ]; then
        echo "rpi5_dispregs: mbox $1 tag 0x00030001 clock $2 status 0x80000000 answered 0x80000008 value $4 rc 0 stage replied"
    else
        echo "rpi5_dispregs: mbox $1 tag 0x00030001 clock $2 status 0x00000000 answered 0x00000000 value 0x00000000 rc -5 stage replied"
    fi
}

# clock <id> <name> <rc> <state> <rate> <measured> <max>
clock() {
    echo "rpi5_dispregs: clock $1 $2"
    tag state "$1" "$3" "$4"
    tag rate "$1" "$3" "$5"
    tag measured "$1" "$3" "$6"
    tag max "$1" "$3" "$7"
}

# yline_lines <mode>: the image's YLINE trace summary (src/main.c
# yline_trace()); mode counts, stuck, fewvals, disagree, skipped or none.
yline_lines() {
    trace="rpi5_dispregs: yline trace ch0 samples 4096 period_ns 14648 span_us 59985 cycle_hz 54000000 all_ones 0 stepping D0"
    case "$1" in
    counts)
        echo "$trace"
        echo "rpi5_dispregs: yline range min 0 max 1124 distinct 1080 wraps 3 max_jump 2"
        echo "rpi5_dispregs: yline frcnt first 5 last 8 steps 3"
        echo "rpi5_dispregs: yline step 0 frcnt 5->6 sample 700 yline 1124->0"
        echo "rpi5_dispregs: yline step 1 frcnt 6->7 sample 1840 yline 1123->1"
        echo "rpi5_dispregs: yline step 2 frcnt 7->8 sample 2980 yline 1124->0"
        echo "rpi5_dispregs: q4yline: counts (1080 distinct values, 3 wraps over 3 FRCNT steps)"
        ;;
    stuck | disagree)
        echo "$trace"
        echo "rpi5_dispregs: yline range min 0 max 0 distinct 1 wraps 0 max_jump 0"
        echo "rpi5_dispregs: yline frcnt first 5 last 5 steps 0"
        if [ "$1" = stuck ]; then
            echo "rpi5_dispregs: q4yline: stuck (YLINE 0 in all 4096 samples)"
        else
            echo "rpi5_dispregs: q4yline: counts (1080 distinct values, 3 wraps over 3 FRCNT steps)"
        fi
        ;;
    fewvals)
        echo "$trace"
        echo "rpi5_dispregs: yline range min 0 max 39 distinct 40 wraps 3 max_jump 1"
        echo "rpi5_dispregs: yline frcnt first 5 last 8 steps 3"
        echo "rpi5_dispregs: yline step 0 frcnt 5->6 sample 700 yline 39->0"
        echo "rpi5_dispregs: yline step 1 frcnt 6->7 sample 1840 yline 39->0"
        echo "rpi5_dispregs: yline step 2 frcnt 7->8 sample 2980 yline 39->0"
        echo "rpi5_dispregs: q4yline: unknown (40 distinct values, 3 wraps, 3 FRCNT steps)"
        ;;
    skipped)
        echo "rpi5_dispregs: yline trace ch0 skipped (the HVS did not map)"
        echo "rpi5_dispregs: q4yline: unknown (no trace: the HVS did not map)"
        ;;
    esac
}

# make_log <file> <pv0 width> <hvs fwidth> <pv0 en> <hsm state> <hsm measured> <mbox rc> <end line> [yline mode, default counts]
# hsm state 0x1 on, 0x0 off. A measured rate of 0x0 means the clock is off.
make_log() {
    f=$1
    pvw=$2
    fw=$3
    pven=$4
    hs=$5
    hm=$6
    rc=$7
    end=$8
    ymode=${9:-counts}
    {
        echo "rpi5_dispregs: build abc1234"
        echo "rpi5_dispregs: cache management on"
        echo "rpi5_dispregs: mailbox mapped"
        echo "rpi5_dispregs: hvs mapped version 0x00000054 stepping D0"
        echo "rpi5_dispregs: hvs ch0 ctrl0 0x80000438 enb 1 fwidth $fw lines 1080 status 0x00052000 mode 2 frcnt 5"
        echo "rpi5_dispregs: hvs ch1 ctrl0 0x00000000 enb 0 fwidth 0 lines 0 status 0x00000000 mode 0 frcnt 0"
        yline_lines "$ymode"
        echo "rpi5_dispregs: pv0 mapped"
        echo "rpi5_dispregs: pv0 regs control 0x00000001 vcontrol 0x00000001 horza 0x00940058 horzb 0x00200780 verta 0x00240005 vertb 0x00040438"
        echo "rpi5_dispregs: pv0 decode en $pven viden $pven clk_select 1 pixel_rep 0 format 0 hactive $pvw hfp 32 hsync 40 hbp 74 vactive 1080 vfp 4 vsync 5 vbp 36 vic 0"
        echo "rpi5_dispregs: pv1 mapped"
        echo "rpi5_dispregs: pv1 regs control 0x00000000 vcontrol 0x00000000 horza 0x00000000 horzb 0x00000000 verta 0x00000000 vertb 0x00000000"
        echo "rpi5_dispregs: pv1 decode en 0 viden 0 clk_select 0 pixel_rep 0 format 0 hactive 0 hfp 0 hsync 0 hbp 0 vactive 0 vfp 0 vsync 0 vbp 0 vic 0"
        echo "rpi5_dispregs: pv mux/stat not read (no offset of them in the repo)"
        clock 4 core "$rc" 0x1 0x1dcd6500 0x1dcd6500 0x2faf0800
        clock 13 hsm "$rc" "$hs" 0x0 "$hm" 0x2faf0800
        clock 14 bvb "$rc" 0x1 0x11e1a300 0x11e1a300 0x11e1a300
        clock 16 disp "$rc" 0x1 0x23c34600 0x23c34600 0x23c34600
        if [ "$rc" = 0 ]; then
            echo "rpi5_dispregs: mbox num_displays tag 0x00040013 status 0x80000000 answered 0x80000004 value 0x00000002 rc 0 stage replied"
            echo "rpi5_dispregs: mbox display_settings tag 0x00040014 status 0x80000000 answered 0x80000040 value 0x00000000 rc 0 stage replied"
        else
            echo "rpi5_dispregs: mbox num_displays tag 0x00040013 status 0x00000000 answered 0x00000000 value 0x00000000 rc -5 stage replied"
            echo "rpi5_dispregs: mbox display_settings tag 0x00040014 status 0x00000000 answered 0x00000000 value 0x00000000 rc -5 stage replied"
        fi
        echo "rpi5_dispregs: display_settings words 0x00000000 0x00000000"
        if [ "$hs" = 0x1 ] && [ "$hm" != 0x0 ]; then
            echo "rpi5_dispregs: hdmi reading (hsm clock on, measured 400000000 Hz)"
            echo "rpi5_dispregs: hdmi0 hotplug 0x00000001 connected 1 hd_vid_ctl 0x80000000 enable 1"
            echo "rpi5_dispregs: hdmi1 hotplug 0x00000000 connected 0 hd_vid_ctl 0x00000000 enable 0"
        else
            echo "rpi5_dispregs: hdmi skipped (hsm clock off)"
        fi
        [ -n "$end" ] && echo "rpi5_dispregs: $end"
        [ -n "$end" ] && echo "rpi5_dispregs: feeding stopped, reset expected (the PM watchdog resets the SoC)"
    } >"$f"
}

run() {
    out=$(sh "$V" "$1" 2>&1)
    rc=$?
}

DONE="done (pv 2 of 2, clock tags 16 of 16, hdmi read)"

# read
make_log "$tmp/read" 960 1920 1 0x1 0x17d78400 0 "$DONE"
run "$tmp/read"
expect_rc read "$rc" 0
expect_has read "$out" "q3: pv0 runs (the repo maps pv0 to HDMI0, pv1 to HDMI1); two pixels per clock: yes (valve active width 960 = HVS FWIDTH 1920 / 2)"
expect_has read "$out" "q3 pv0: running clk_select 1 pixel_rep 0 active 960x1080"
expect_has read "$out" "q3 pv1: stopped"
expect_has read "$out" "q5 clock hsm (13): state on rate 0 measured 400000000 max 800000000"
expect_has read "$out" "q5 clock disp (16): state on rate 600000000 measured 600000000 max 600000000"
expect_has read "$out" "q5: hsm on measured 400000000, bvb on measured 300000000, disp on measured 600000000"
expect_has read "$out" "q6: displays: 2, settings: answered, hdmi: hdmi0 hotplug 1 hd_vid_ctl enable 1, hdmi1 hotplug 0 hd_vid_ctl enable 0; the firmware drives HDMI0"
expect_has read "$out" "end: done ("
expect_has read "$out" "verdict: done"
expect_has read "$out" "q4yline: counts (samples 4096 period_ns 14648 span_us 59985, yline 0..1124 distinct 1080 wraps 3 max_jump 2, frcnt 5..8 steps 3)"

# skipped: the HSM clock is off
make_log "$tmp/skipped" 960 1920 1 0x0 0x0 0 "done (pv 2 of 2, clock tags 16 of 16, hdmi skipped)"
run "$tmp/skipped"
expect_rc skipped "$rc" 0
expect_has skipped "$out" "q5 clock hsm (13): state off"
expect_has skipped "$out" "q6: displays: 2, settings: answered, hdmi: skipped (hsm clock off)"
expect_has skipped "$out" "q3: pv0 runs"

# onepx: one pixel per clock
make_log "$tmp/onepx" 1920 1920 1 0x1 0x17d78400 0 "$DONE"
run "$tmp/onepx"
expect_rc onepx "$rc" 0
expect_has onepx "$out" "two pixels per clock: no (valve active width 1920 = HVS FWIDTH 1920)"

# odd widths
make_log "$tmp/odd" 800 1920 1 0x1 0x17d78400 0 "$DONE"
run "$tmp/odd"
expect_has odd "$out" "two pixels per clock: unknown (valve active width 800, HVS FWIDTH 1920)"

# stopped valves
make_log "$tmp/stopped" 0 0 0 0x1 0x17d78400 0 "$DONE"
run "$tmp/stopped"
expect_rc stopped "$rc" 0
expect_has stopped "$out" "q3: no pixel valve runs"

# every mailbox tag failed
make_log "$tmp/noans" 960 1920 1 0x0 0x0 1 "done (pv 2 of 2, clock tags 0 of 16, hdmi skipped)"
run "$tmp/noans"
expect_rc noans "$rc" 0
expect_has noans "$out" "q5 clock disp (16): state not answered rate - measured - max -"
expect_has noans "$out" "q6: displays: not answered, settings: not answered, hdmi: skipped (hsm clock off)"

# FAIL
make_log "$tmp/fail" 960 1920 1 0x1 0x17d78400 0 "FAIL (no mailbox mapping)"
run "$tmp/fail"
expect_rc fail "$rc" 3
expect_has fail "$out" "verdict: FAIL (no mailbox mapping)"

# incomplete
make_log "$tmp/inc" 960 1920 1 0x1 0x17d78400 0 ""
run "$tmp/inc"
expect_rc incomplete "$rc" 1
expect_has incomplete "$out" "end: missing"
expect_has incomplete "$out" "verdict: incomplete (no end line)"
expect_has incomplete "$out" "q3: pv0 runs"

# empty log: every answer line still printed
: >"$tmp/empty"
run "$tmp/empty"
expect_rc empty "$rc" 1
expect_has empty "$out" "q3: unknown (no pixel valve line in the log)"
expect_has empty "$out" "q5 clock hsm (13): state not answered (no line in the log)"
expect_has empty "$out" "q6: displays: missing (no line in the log), settings: missing, hdmi: missing (no hdmi line in the log)"

# crlf and a timestamp prefix
sed 's/^/[00:00:01.000,000] <inf> app: /; s/$/\r/' "$tmp/read" >"$tmp/crlf"
run "$tmp/crlf"
expect_rc crlf "$rc" 0
expect_has crlf "$out" "two pixels per clock: yes"
expect_has crlf "$out" "the firmware drives HDMI0"
expect_has crlf "$out" "q4yline: counts"

# stuck: YLINE never moves
make_log "$tmp/stuck" 960 1920 1 0x1 0x17d78400 0 "$DONE" stuck
run "$tmp/stuck"
expect_rc stuck "$rc" 0
expect_has stuck "$out" "q4yline: stuck (samples 4096"
expect_has stuck "$out" "distinct 1 wraps 0"
expect_has stuck "$out" "verdict: done"

# fewvals: YLINE moves over too few values for counts
make_log "$tmp/fewvals" 960 1920 1 0x1 0x17d78400 0 "$DONE" fewvals
run "$tmp/fewvals"
expect_rc fewvals "$rc" 0
expect_has fewvals "$out" "q4yline: unknown (samples 4096"

# missing: done but no trace is a FAIL
make_log "$tmp/missing" 960 1920 1 0x1 0x17d78400 0 "$DONE" none
run "$tmp/missing"
expect_rc missing "$rc" 3
expect_has missing "$out" "q4yline: missing (no yline trace line in the log)"
expect_has missing "$out" "verdict: FAIL (the image said done but printed no yline trace)"

# missing without an end line: incomplete, not FAIL
make_log "$tmp/missing-inc" 960 1920 1 0x1 0x17d78400 0 "" none
run "$tmp/missing-inc"
expect_rc missing-inc "$rc" 1
expect_has missing-inc "$out" "q4yline: missing"
expect_has missing-inc "$out" "verdict: incomplete (no end line)"

# missing with the image's FAIL end line: the image's reason stands
make_log "$tmp/missing-fail" 960 1920 1 0x1 0x17d78400 0 "FAIL (the HVS did not map)" none
run "$tmp/missing-fail"
expect_rc missing-fail "$rc" 3
expect_has missing-fail "$out" "verdict: FAIL (the HVS did not map)"

# disagree: the image's verdict does not follow from its own summary
make_log "$tmp/disagree" 960 1920 1 0x1 0x17d78400 0 "$DONE" disagree
run "$tmp/disagree"
expect_rc disagree "$rc" 3
expect_has disagree "$out" "verdict: FAIL (q4yline: the image says counts, its summary gives stuck)"

# yskip: the image skipped the trace
make_log "$tmp/yskip" 960 1920 1 0x1 0x17d78400 0 "$DONE" skipped
run "$tmp/yskip"
expect_rc yskip "$rc" 0
expect_has yskip "$out" "q4yline: unknown (the image skipped the trace: unknown (no trace: the HVS did not map))"

# usage
out=$(sh "$V" 2>&1)
expect_rc usage-noarg "$?" 2
out=$(sh "$V" "$tmp/nonexistent" 2>&1)
expect_rc usage-unreadable "$?" 2

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
