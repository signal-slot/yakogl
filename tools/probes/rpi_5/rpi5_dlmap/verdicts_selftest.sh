#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_dlmap/verdicts_selftest.sh
#
# Feeds verdicts.sh canned rpi5_dlmap console logs, written in the image's
# line format (src/main.c), and checks the fwend:/q4place: lines and the exit
# status of each:
#
#   after      channel 0's walk ends at an END (word 1210), a zero span
#              follows: fwend word 1210, q4place after-end 1216 (exit 0)
#   noend      shaped like the 2026-11-25 numbers: HEADE 672, eight entries of
#              NEXT 32 up to word 896 (VALID clear), words 32..671 with no
#              zero span of 20, the walk then ends at NEXT 0 (exit 0;
#              fwend none, q4place none)
#   before     no END, but a zero span of 24 words below HEADE (q4place
#              before-head 100, exit 0)
#   endfull    END found but no zero span after it, a span below HEADE
#              (q4place before-head)
#   endnone    END found, nothing after it and nothing below HEADE (none)
#   clamp      a zero span below HEADE that starts under word 32 is clamped
#              to word 32 (before-head 32)
#   fail       the image's FAIL end line (exit 3)
#   nomem      done without a mem_words line (FAIL, exit 3)
#   incomplete the log stops before the end line (exit 1)
#   crlf       the "after" log with CR line ends and a timestamp prefix
#   usage      no argument, an unreadable file (exit 2)
#
# Prints one line per failed check and a last line
# "verdicts_selftest: N checks, M failed"; exits 0 when none failed.
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_dlmap_verdicts.XXXXXX") || exit 1
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

# expect_not <name> <output> <substring>: the output does not hold it.
expect_not() {
    checks=$((checks + 1))
    case "$2" in
    *"$3"*) fail "$1: unexpected '$3'" ;;
    *) ;;
    esac
}

# expect_rc <name> <got> <want>
expect_rc() {
    checks=$((checks + 1))
    [ "$2" = "$3" ] || fail "$1: exit $2, wanted $3"
}

# head_lines <heade>: the preamble and the three channel lines.
head_lines() {
    echo "rpi5_dlmap: build 00000000"
    echo "rpi5_dlmap: read-only: no HVS register and no list word is written"
    echo "rpi5_dlmap: hvs version 0x00002454 stepping D0 cxm_size 0x00006000"
    echo "rpi5_dlmap: ch0 ctrl0 0x8780077f lptrs 0x00000$(printf '%x' "$1") heade $1 dl 0x000002a0 status 0x00006a45"
    echo "rpi5_dlmap: ch1 ctrl0 0x8279011f lptrs 0x00000$(printf '%x' "$1") heade $1 dl 0x000002a0 status 0x00004000"
    echo "rpi5_dlmap: ch2 ctrl0 0x00000000 lptrs 0x00000000 heade 0 dl 0x00000000 status 0x00000000"
    echo "rpi5_dlmap: mem_words 24576 (CXM_SIZE 24576, mapped list words 24576)"
}

# walk8 <ch> <first>: eight NEXT 32 entries from <first>, VALID clear on the last.
walk8() {
    w=$2
    k=0
    while [ "$k" -lt 8 ]; do
        v=1
        c=0x72000000
        if [ "$k" -eq 7 ]; then
            v=0
            c=0x20000000
        fi
        echo "rpi5_dlmap: walk ch$1 entry $k word $w ctl0 $c valid $v next 32"
        w=$((w + 32))
        k=$((k + 1))
    done
}

# fixture <name>: write $tmp/<name>.log
mk() {
    f="$tmp/$1.log"
    case "$1" in
    after)
        {
            head_lines 672
            echo "rpi5_dlmap: span 0..1209 n=1210"
            echo "rpi5_dlmap: zero 1216..24575"
            echo "rpi5_dlmap: end-word 1210 0x80000000"
            walk8 0 672
            echo "rpi5_dlmap: walk ch0 entry 8 word 928 ctl0 0x80000000 valid 0 next 0"
            echo "rpi5_dlmap: walk ch0 stop END at word 1210"
            echo "rpi5_dlmap: walk ch1 stop NEXT 0 at word 672"
            echo "rpi5_dlmap: walk ch2 stop NEXT 0 at word 0"
            echo "rpi5_dlmap: after-end zero 1216..24575"
            echo "rpi5_dlmap: done (mem_words 24576, channel 0 END found)"
            echo "rpi5_dlmap: feeding stopped, reset expected"
        } >"$f"
        ;;
    noend)
        {
            head_lines 672
            echo "rpi5_dlmap: span 0..6 n=7"
            echo "rpi5_dlmap: span 32..671 n=640"
            echo "rpi5_dlmap: span 672..927 n=256"
            echo "rpi5_dlmap: zero 1000..24575"
            walk8 0 672
            echo "rpi5_dlmap: walk ch0 entry 8 word 928 ctl0 0x00000000 valid 0 next 0"
            echo "rpi5_dlmap: walk ch0 stop NEXT 0 at word 928"
            echo "rpi5_dlmap: after-end none (channel 0's walk found no END)"
            echo "rpi5_dlmap: done (mem_words 24576, channel 0 END not found)"
        } >"$f"
        ;;
    before)
        {
            head_lines 672
            echo "rpi5_dlmap: zero 100..123"
            echo "rpi5_dlmap: zero 1000..24575"
            walk8 0 672
            echo "rpi5_dlmap: walk ch0 stop step bound at word 928"
            echo "rpi5_dlmap: after-end none (channel 0's walk found no END)"
            echo "rpi5_dlmap: done (mem_words 24576, channel 0 END not found)"
        } >"$f"
        ;;
    endfull)
        {
            head_lines 672
            echo "rpi5_dlmap: zero 40..70"
            echo "rpi5_dlmap: end-word 700 0x80000000"
            echo "rpi5_dlmap: walk ch0 entry 0 word 672 ctl0 0x72000000 valid 1 next 32"
            echo "rpi5_dlmap: walk ch0 entry 1 word 704 ctl0 0x80000000 valid 0 next 0"
            echo "rpi5_dlmap: walk ch0 stop END at word 704"
            echo "rpi5_dlmap: after-end none (no zero span of 20 words after word 704)"
            echo "rpi5_dlmap: done (mem_words 24576, channel 0 END found)"
        } >"$f"
        ;;
    endnone)
        {
            head_lines 672
            echo "rpi5_dlmap: zero 40..50"
            echo "rpi5_dlmap: walk ch0 entry 0 word 672 ctl0 0x80000000 valid 0 next 0"
            echo "rpi5_dlmap: walk ch0 stop END at word 672"
            echo "rpi5_dlmap: after-end none (no zero span of 20 words after word 672)"
            echo "rpi5_dlmap: done (mem_words 24576, channel 0 END found)"
        } >"$f"
        ;;
    clamp)
        {
            head_lines 672
            echo "rpi5_dlmap: zero 10..60"
            walk8 0 672
            echo "rpi5_dlmap: walk ch0 stop past memory at word 30000"
            echo "rpi5_dlmap: after-end none (channel 0's walk found no END)"
            echo "rpi5_dlmap: done (mem_words 24576, channel 0 END not found)"
        } >"$f"
        ;;
    fail)
        {
            echo "rpi5_dlmap: build 00000000"
            echo "rpi5_dlmap: FAIL (the HVS did not map)"
        } >"$f"
        ;;
    nomem)
        {
            echo "rpi5_dlmap: ch0 ctrl0 0x8780077f lptrs 0x000002a0 heade 672 dl 0x000002a0 status 0x00006a45"
            echo "rpi5_dlmap: walk ch0 stop NEXT 0 at word 672"
            echo "rpi5_dlmap: done (mem_words 0, channel 0 END not found)"
        } >"$f"
        ;;
    incomplete)
        {
            head_lines 672
            echo "rpi5_dlmap: zero 1000..24575"
        } >"$f"
        ;;
    crlf)
        mk after
        sed 's/^/12:34:56 /; s/$/\r/' "$tmp/after.log" >"$tmp/crlf.log"
        ;;
    esac
}

run() {
    mk "$1"
    out=$(sh "$V" "$tmp/$1.log" 2>&1)
    rc=$?
}

run after
expect_rc after "$rc" 0
expect_has after "$out" "fwend: word 1210"
expect_has after "$out" "q4place: after-end 1216"
expect_has after "$out" "ch0 heade 672: 9 entries, stop END at word 1210"
expect_has after "$out" "verdict: done"

run noend
expect_rc noend "$rc" 0
expect_has noend "$out" "fwend: none"
expect_has noend "$out" "q4place: none ("
expect_not noend "$out" "before-head"
expect_not noend "$out" "after-end"
expect_has noend "$out" "ch0 heade 672: 9 entries, stop NEXT 0 at word 928"
expect_has noend "$out" "mem_words 24576 (spans 3, zero spans 1"

run before
expect_rc before "$rc" 0
expect_has before "$out" "fwend: none"
expect_has before "$out" "q4place: before-head 100"
expect_has before "$out" "stop step bound at word 928"

run endfull
expect_rc endfull "$rc" 0
expect_has endfull "$out" "fwend: word 704"
expect_has endfull "$out" "q4place: before-head 40"

run endnone
expect_rc endnone "$rc" 0
expect_has endnone "$out" "fwend: word 672"
expect_has endnone "$out" "q4place: none (no zero span of 20 words after the END at word 672"

run clamp
expect_rc clamp "$rc" 0
expect_has clamp "$out" "q4place: before-head 32"
expect_has clamp "$out" "stop past memory at word 30000"

run fail
expect_rc fail "$rc" 3
expect_has fail "$out" "FAIL (the HVS did not map)"
expect_has fail "$out" "fwend: none"

run nomem
expect_rc nomem "$rc" 3
expect_has nomem "$out" "FAIL (the image said done but the log lacks mem_words)"

run incomplete
expect_rc incomplete "$rc" 1
expect_has incomplete "$out" "incomplete (no end line)"
expect_has incomplete "$out" "end: missing"

run crlf
expect_rc crlf "$rc" 0
expect_has crlf "$out" "q4place: after-end 1216"

sh "$V" >/dev/null 2>&1
expect_rc usage-noarg "$?" 2
sh "$V" "$tmp/does-not-exist.log" >/dev/null 2>&1
expect_rc usage-missing "$?" 2
sh "$V" "$tmp" >/dev/null 2>&1
expect_rc usage-dir "$?" 2

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
