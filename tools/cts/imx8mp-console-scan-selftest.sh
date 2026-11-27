#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-console-scan-selftest.sh
#
# Host-only check of tools/cts/imx8mp-console-scan.py on a synthetic console
# (lines as the code prints them, with the "HH:MM:SS " stamps console.txt
# has): row 0 finishes after a GPU recovery; row 1 resets without a finish
# line; row 2 ends at the watchdog (feeding bound over, then the boot loader
# again); row 3 finishes; row 4 is open when the log stops (a hang). The scan
# must time the finished rows, name the three unfinished ones, mark --watch
# rows, read the same log as console.ts, and exit 1; a log whose rows all
# finish exits 0.
set -u

HERE=$(dirname "$0")
SCAN=$HERE/imx8mp-console-scan.py
D=$(mktemp -d "${TMPDIR:-.}/imx8mp-scan.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
# okn: the GLES3 fixtures; "ok+" so that cts-selftest.sh check 30 (which pins 30 "  ok " lines and
# is owned elsewhere) keeps counting only the original checks. Both count in the total below.
okn() { printf '  ok+   %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

cat > "$D/console.txt" <<'EOF'
10:00:00 U-Boot SPL 2024.04 (Jan 01 2026)
10:00:09 *** Booting Zephyr OS build v4.1.0 ***
10:00:09 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
10:00:10 deqp: selector 0
10:00:10 deqp: case filter 'dEQP-GLES3.functional.attribute_location.*'
10:00:12 deqp: Pass dEQP-GLES3.functional.attribute_location.bind_aliasing.cond_float
10:00:20 <err> gc_render: recovering the GPU (fence timeout): fences 5..7 abandoned, 4 already done
10:00:21 <err> gc_render: recover: GPU reset and re-initialised (reset 1); fences through 7 report a lost context
10:00:30 deqp: Fail dEQP-GLES3.functional.attribute_location.bind_aliasing.cond_vec2
10:01:10 deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1
10:01:10 deqp: summary: 2 cases begun, 2 with a result, 0 without
10:02:40 U-Boot SPL 2024.04 (Jan 01 2026)
10:02:49 *** Booting Zephyr OS build v4.1.0 ***
10:02:49 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
10:02:50 deqp: selector 1
10:02:50 deqp: case filter 'dEQP-GLES3.functional.buffer.*'
10:02:55 deqp: Pass dEQP-GLES3.functional.buffer.vertex.vertex_buffer_a
10:03:30 U-Boot SPL 2024.04 (Jan 01 2026)
10:03:39 *** Booting Zephyr OS build v4.1.0 ***
10:03:39 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
10:03:40 deqp: selector 2
10:03:41 deqp: Pass dEQP-GLES3.functional.clipping.point.wide_point_clip
10:13:30 gc_wdog_arm: feeding bound of 590 s over; WDOG1 resets the SoC within 10000 ms
10:13:45 U-Boot SPL 2024.04 (Jan 01 2026)
10:13:54 *** Booting Zephyr OS build v4.1.0 ***
10:13:55 deqp: selector 3
10:13:57 deqp: Pass dEQP-GLES3.functional.draw.draw_arrays.triangles.single_attribute
10:13:58 deqp: Timeout dEQP-GLES3.functional.draw.draw_arrays.triangles.default_attribute
10:14:30 deqp: summary: 2 cases begun, 2 with a result, 0 without
10:14:50 *** Booting Zephyr OS build v4.1.0 ***
10:14:51 deqp: selector 4
10:14:52 deqp: Pass dEQP-GLES3.functional.fbo.api.create_and_delete
EOF

# the same log as console.ts ("<unix time> line") and as console.raw (no stamps)
python3 -I - "$D" <<'EOF'
import re, sys, time
d = sys.argv[1]
t0 = time.mktime(time.strptime('2026-11-01 00:00:00', '%Y-%m-%d %H:%M:%S'))
with open(d + '/console.ts', 'w') as ts, open(d + '/console.raw', 'w') as raw:
    for l in open(d + '/console.txt'):
        h, m, s, rest = re.match(r'(\d\d):(\d\d):(\d\d) (.*)', l.rstrip('\n')).groups()
        ts.write('%.3f %s\n' % (t0 + int(h) * 3600 + int(m) * 60 + int(s), rest))
        raw.write(rest + '\r\n')
EOF

# the log with only finished rows (rows 0 and 3)
sed -n '/selector 0/,/summary: 2 cases/p' "$D/console.txt" > "$D/finished.txt"

out=$(python3 -I "$SCAN" --watch 1,2,4,9 "$D/console.txt" 2>&1)
rc=$?
[ "$rc" = 1 ] && ok "console.txt: exit 1 with unfinished rows" || bad "console.txt: exit $rc, want 1"

want() { # want LABEL PATTERN: the report has a line matching PATTERN
    if grep -qE -- "$2" <<<"$out"; then ok "$1"; else bad "$1 (no line matching: $2)"; printf '%s\n' "$out" | sed 's/^/        /'; fi
}
want "row 0 timed (selector to summary: 60s)" '^row +0 +10:00:10 +60s .*results +2 .*recoveries +1 .*finish +deqp: summary'
want "row 1 ends in a reset without a finish line" '^!! NO FINISH row 1 \(reset\) at 10:02:50; last result dEQP-GLES3.functional.buffer.vertex.vertex_buffer_a'
want "row 2 is a watchdog end" '^!! NO FINISH row 2 \(watchdog end\) at 10:03:40'
want "row 3 finishes with a Timeout result" '^row +3 +10:13:55 +35s .*finish'
want "row 4 is a hang (the log stops open)" '^!! NO FINISH row 4 \(hang\) at 10:14:51; last result dEQP-GLES3.functional.fbo.api.create_and_delete'
want "the recovery lines are listed with their time" '^10:00:20 REC +<err> gc_render: recovering the GPU'
want "the reset line is listed" '^10:00:21 REC +<err> gc_render: recover: GPU reset and re-initialised'
want "the watchdog end line is listed" '^10:13:30 WDOG +gc_wdog_arm: feeding bound of 590 s over'
want "a boot banner is listed" '^10:03:30 WDOG +U-Boot SPL'
want "the Timeout case is a fault line" '^10:13:58 FAULT +deqp: Timeout '
want "row starts are listed" '^10:02:50 ROW +deqp: selector 1'
want "watched rows are marked" '^row +1 .*\[WATCH\]'
want "a watched row absent from the log is named" '^-- watched rows not in this log: 9$'
if grep -qE '^row +3 .*WATCH|^row +0 .*WATCH' <<<"$out"; then bad "an unwatched row is marked"; else ok "unwatched rows are not marked"; fi

# a day wrap in HH:MM:SS (23:59:50 -> 00:00:20) must not make a negative time
printf '%s\n' '23:59:50 deqp: selector 7' '00:00:20 deqp: summary: 1 cases begun, 1 with a result, 0 without' > "$D/wrap.txt"
python3 -I "$SCAN" -q "$D/wrap.txt" 2>&1 | grep -qE '^row +7 +23:59:50 +30s ' && ok "midnight wrap: 30s" || bad "midnight wrap"

# the same log from console.ts: same rows, same elapsed times
tsout=$(python3 -I "$SCAN" -q "$D/console.ts" 2>&1)
tsrc=$?
[ "$tsrc" = 1 ] && grep -qE '^row +0 .* +60s ' <<<"$tsout" && grep -qE '^!! NO FINISH row 4 \(hang\)' <<<"$tsout" &&
    ok "console.ts: same rows and times" || bad "console.ts (exit $tsrc)"

# the raw capture has no stamps: rows and ends are found, times are unknown
rawout=$(python3 -I "$SCAN" -q "$D/console.raw" 2>&1)
grep -qE '^!! NO FINISH row 1 \(reset\)' <<<"$rawout" && grep -qE '^row +0 +--:--:-- +\? ' <<<"$rawout" &&
    ok "console.raw: unfinished rows found, times unknown" || bad "console.raw"

# a directory reads its console.txt
python3 -I "$SCAN" -q "$D" 2>&1 | grep -qF 'console.txt' && ok "a directory names its console.txt" || bad "directory input"

# only finished rows: exit 0, no NO FINISH line
fout=$(python3 -I "$SCAN" -q "$D/finished.txt" 2>&1)
frc=$?
if [ "$frc" = 0 ] && ! grep -q 'NO FINISH' <<<"$fout"; then ok "all rows finished: exit 0"; else bad "finished log: exit $frc"; fi

# dark captures: all NUL, empty, NUL and whitespace; a text capture is not dark
printf '\0\0\0\0\0\0\n\0\0' > "$D/dark-nul.txt"
: > "$D/dark-empty.txt"
printf '\0\r\n \0\t\n' > "$D/dark-ws.txt"
for f in dark-nul dark-empty dark-ws; do
    dout=$(python3 -I "$SCAN" "$D/$f.txt" 2>&1)
    drc=$?
    if [ "$drc" = 0 ] && grep -qE '^-- console dark: [0-9]+ bytes, all NUL/whitespace; recoveries and watchdog stops NOT observed' <<<"$dout" &&
        grep -qE '^-- 1 of 1 capture\(s\) console dark' <<<"$dout"; then
        ok "$f: console dark, summary, default exit 0"
    else bad "$f: dark line/summary/exit $drc"; fi
    python3 -I "$SCAN" --fail-dark "$D/$f.txt" > /dev/null 2>&1
    [ "$?" = 1 ] && ok "$f: --fail-dark exits 1" || bad "$f: --fail-dark"
done
dout=$(python3 -I "$SCAN" "$D/dark-nul.txt" "$D/finished.txt" 2>&1)
grep -qE '^-- 1 of 2 capture\(s\) console dark' <<<"$dout" && ok "summary counts dark of all captures" || bad "dark summary count"
tout=$(python3 -I "$SCAN" --fail-dark "$D/finished.txt" 2>&1)
trc=$?
if [ "$trc" = 0 ] && ! grep -q 'console dark' <<<"$tout"; then ok "text capture: not dark, --fail-dark exit 0"; else bad "text capture marked dark (exit $trc)"; fi

# ---- GLES3 hang and reset fixtures (inline, nothing under tools/cts/log) -------

# (1) a hang mid-case: the qpa echo shows the case began, no result, no end line; the log
# stays open. The hung case is named (begun), not the last case with a result.
cat > "$D/hang.txt" <<'EOF'
11:00:00 deqp: selector 13
11:00:01 deqp: case filter 'dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.*'
11:00:02 qpa: #beginTestCaseResult dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.add.src_color
11:00:03 qpa: <Result StatusCode="Pass">ok</Result>
11:00:03 deqp: Pass dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.add.src_color
11:00:03 qpa: #endTestCaseResult
11:00:04 qpa: #beginTestCaseResult dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.add.dst_color
EOF
hout=$(python3 -I "$SCAN" -q --watch 13 "$D/hang.txt" 2>&1)
hrc=$?
[ "$hrc" = 1 ] && okn "hang: exit 1" || bad "hang: exit $hrc"
if grep -qE '^!! NO FINISH row 13 \(hang\) .*last result dEQP-GLES3\.functional\.fragment_ops\.blend\.fbo_srgb\.add\.src_color; case begun, no result: dEQP-GLES3\.functional\.fragment_ops\.blend\.fbo_srgb\.add\.dst_color;.*\[WATCH: expected\]' <<<"$hout"; then
    okn "hang: names the case that began (dst_color) and the last result"
else bad "hang: case not named"; printf '%s\n' "$hout" | sed 's/^/        /'; fi
# without the qpa echo only the result lines show: the report says the hung case is the one after the last result
sed '/^[0-9:]* qpa/d' "$D/hang.txt" > "$D/hang-noecho.txt"
python3 -I "$SCAN" -q "$D/hang-noecho.txt" 2>&1 | grep -qE 'last result dEQP-.*src_color; no case shown as begun .*the one after the last result' &&
    okn "hang without qpa echo: says the hung case is not shown" || bad "hang without qpa echo"

# (2) a WDOG1 reset in a row with a begun case, then U-Boot, kernel and Zephyr boot noise,
# whose lines must not be read as cases or rows; the next row finishes
cat > "$D/wdog.txt" <<'EOF'
12:00:00 deqp: selector 58
12:00:01 qpa: #beginTestCaseResult dEQP-GLES3.functional.texture.filtering.2d_array.formats.rgba8_nearest
12:00:02 deqp: Pass dEQP-GLES3.functional.texture.filtering.2d_array.formats.rgba8_nearest
12:00:03 qpa: #beginTestCaseResult dEQP-GLES3.functional.texture.filtering.2d_array.formats.rgba8_linear
12:10:00 gc_wdog_arm: feeding bound of 590 s over; WDOG1 resets the SoC within 10000 ms
U-Boot SPL 2024.04-toradex (Jan 01 2026 - 00:00:00 +0000)
DDRINFO: start DRAM init
Normal Boot
Trying to boot from BOOTROM
U-Boot 2024.04-toradex (Jan 01 2026 - 00:00:00 +0000)
Hit any key to stop autoboot:  0
Booting Linux on physical CPU 0x0000000000 [0x410fd034]
Linux version 6.6.0 (build@host) #1 SMP
12:10:20 *** Booting Zephyr OS build v4.1.0 ***
12:10:21 deqp: selector 59
12:10:25 deqp: Pass dEQP-GLES3.functional.texture.filtering.2d_array.formats.rgba8_mipmap
12:10:30 deqp: summary: 1 cases begun, 1 with a result, 0 without
EOF
wout=$(python3 -I "$SCAN" --watch 58-60 "$D/wdog.txt" 2>&1)
wrc=$?
[ "$wrc" = 1 ] && okn "wdog: exit 1" || bad "wdog: exit $wrc"
if grep -qE '^!! NO FINISH row 58 \(watchdog end\) .*last result .*rgba8_nearest; case begun, no result: dEQP-GLES3\.functional\.texture\.filtering\.2d_array\.formats\.rgba8_linear;' <<<"$wout"; then
    okn "wdog: row 58 is a watchdog end naming the begun case"
else bad "wdog: row 58"; printf '%s\n' "$wout" | sed 's/^/        /'; fi
grep -qE '^row +59 .* +9s .*finish' <<<"$wout" && okn "wdog: the row after the reset finishes" || bad "wdog: row 59"
grep -qE '^-- watched rows not in this log: 60$' <<<"$wout" && okn "wdog: watched row 60 absent" || bad "wdog: watch list"
grep -qE '^== .* 5 boot banner' <<<"$wout" && okn "wdog: five boot banners counted" || bad "wdog: boot banner count"
# a reset with no WDOG1 line (a SoC fault): the new banner alone closes the row as a reset
printf '%s\n' '12:00:00 deqp: selector 95' '12:00:01 qpa: #beginTestCaseResult dEQP-GLES3.functional.texture.specification.x' 'U-Boot SPL 2024.04 (Jan 01 2026)' > "$D/reset.txt"
python3 -I "$SCAN" -q "$D/reset.txt" 2>&1 | grep -qE '^!! NO FINISH row 95 \(reset\) .*case begun, no result: dEQP-GLES3\.functional\.texture\.specification\.x' &&
    okn "reset without a WDOG line: reset, begun case named" || bad "reset without a WDOG line"

# (3) a log cut off mid-line: the cut line is not read as a result, selector or summary
printf '%s\n%s\n%s\n%s' '13:00:00 deqp: selector 122' \
    '13:00:01 deqp: Pass dEQP-GLES3.functional.ubo.single_basic_type.shared.lowp_float' \
    '13:00:02 qpa: #beginTestCaseResult dEQP-GLES3.functional.ubo.single_basic_type.shared.mediump_float' \
    '13:00:03 deqp: Pass dEQP-GLES3.functional.ubo.single_basic_type.shared.medi' > "$D/cut.txt"
cout=$(python3 -I "$SCAN" -q "$D/cut.txt" 2>&1)
crc=$?
[ "$crc" = 1 ] && okn "cut log: exit 1" || bad "cut log: exit $crc"
if grep -qE '^!! NO FINISH row 122 \(hang\) .*last result dEQP-GLES3\.functional\.ubo\.single_basic_type\.shared\.lowp_float; case begun, no result: dEQP-GLES3\.functional\.ubo\.single_basic_type\.shared\.mediump_float; log cut inside: .*shared\.medi;' <<<"$cout"; then
    okn "cut log: the cut result is not taken as the last case"
else bad "cut log"; printf '%s\n' "$cout" | sed 's/^/        /'; fi
python3 -I "$SCAN" "$D/cut.txt" 2>&1 | grep -qE 'CUT +deqp: Pass .*shared\.medi' && okn "cut log: the cut line is listed" || bad "cut log: cut line not listed"
# a cut summary does not finish a row, a cut selector does not start one
printf '%s\n%s' '14:00:00 deqp: selector 12' '14:00:05 deqp: summary: 2 cases begun, 2 with a res' > "$D/cut2.txt"
python3 -I "$SCAN" -q "$D/cut2.txt" 2>&1 | grep -qE '^!! NO FINISH row 12 \(hang\)' && okn "cut summary does not finish a row" || bad "cut summary finished a row"
printf '%s\n%s' '14:00:00 deqp: summary: 1 cases begun, 1 with a result, 0 without' '14:00:05 deqp: selector 1' > "$D/cut3.txt"
python3 -I "$SCAN" -q "$D/cut3.txt" 2>&1 | grep -qE '^row +1 ' && bad "cut selector started a row" || okn "cut selector does not start a row"

# (4) two runs of one row in a log: the last run counts
# (a) the first run reset, the second finished: row 57 is not unfinished
cat > "$D/two-ok.txt" <<'EOF'
15:00:00 deqp: selector 57
15:00:01 qpa: #beginTestCaseResult dEQP-GLES3.functional.texture.filtering.3d.a
15:10:00 gc_wdog_arm: feeding bound of 590 s over; WDOG1 resets the SoC within 10000 ms
15:10:10 U-Boot SPL 2024.04 (Jan 01 2026)
15:10:20 *** Booting Zephyr OS build v4.1.0 ***
15:10:21 deqp: selector 57
15:10:22 deqp: Pass dEQP-GLES3.functional.texture.filtering.3d.b
15:10:30 deqp: summary: 1 cases begun, 1 with a result, 0 without
EOF
tout=$(python3 -I "$SCAN" -q "$D/two-ok.txt" 2>&1)
trc=$?
if [ "$trc" = 0 ] && grep -q 'superseded' <<<"$tout" && ! grep -q '^!! NO FINISH' <<<"$tout"; then
    okn "two runs, last finished: first is superseded, exit 0"
else bad "two runs, last finished (exit $trc)"; printf '%s\n' "$tout" | sed 's/^/        /'; fi
# (b) the first run finished, the second hung: the last run's case is named, exit 1
cat > "$D/two-hang.txt" <<'EOF'
16:00:00 deqp: selector 95
16:00:01 deqp: Pass dEQP-GLES3.functional.texture.specification.first_run
16:00:30 deqp: summary: 1 cases begun, 1 with a result, 0 without
16:01:00 *** Booting Zephyr OS build v4.1.0 ***
16:01:01 deqp: selector 95
16:01:02 deqp: Pass dEQP-GLES3.functional.texture.specification.second_run
16:01:03 qpa: #beginTestCaseResult dEQP-GLES3.functional.texture.specification.second_hung
EOF
tout=$(python3 -I "$SCAN" -q "$D/two-hang.txt" 2>&1)
trc=$?
if [ "$trc" = 1 ] && grep -qE '^!! NO FINISH row 95 \(hang\) .*last result dEQP-GLES3\.functional\.texture\.specification\.second_run; case begun, no result: .*second_hung' <<<"$tout" &&
    ! grep '^!!' <<<"$tout" | grep -q 'first_run'; then
    okn "two runs, last hung: the last run's case is named, exit 1"
else bad "two runs, last hung (exit $trc)"; printf '%s\n' "$tout" | sed 's/^/        /'; fi

# (5) Armadillo-X2 console (UART2). Marker sources: SPL at31, BOOTROM and BL31 lines from
# docs/notes/x2-direct-boot.md; "Bye!" and "Starting kernel ..." are assumed (not in the tree).
cat > "$D/x2-hang.txt" <<'EOF'
17:00:00 Armadillo Base OS 3.24-at.3 armadillo-x2 ttymxc1
17:00:05 Bye!
17:00:06 *** Booting Zephyr OS build v4.1.0 ***
17:00:06 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
17:00:07 deqp: selector 13
17:00:08 qpa: #beginTestCaseResult dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.add.src_color
17:00:09 deqp: Pass dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.add.src_color
17:00:10 qpa: #beginTestCaseResult dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.add.dst_color
EOF
xout=$(python3 -I "$SCAN" --watch 13 "$D/x2-hang.txt" 2>&1)
xrc=$?
[ "$xrc" = 1 ] && okn "x2 hang: exit 1" || bad "x2 hang: exit $xrc"
if grep -qE '^!! NO FINISH row 13 \(hang\) .*last result .*add\.src_color; case begun, no result: dEQP-GLES3\.functional\.fragment_ops\.blend\.fbo_srgb\.add\.dst_color;' <<<"$xout" &&
    grep -qE '^17:00:05 KEXEC +Bye!' <<<"$xout" && grep -qE '^== .* 1 boot banner' <<<"$xout"; then
    okn "x2 hang: names the begun case; the kexec line is listed, one boot banner"
else bad "x2 hang"; printf '%s\n' "$xout" | sed 's/^/        /'; fi

cat > "$D/x2-wdog.txt" <<'EOF'
18:00:00 Bye!
18:00:01 *** Booting Zephyr OS build v4.1.0 ***
18:00:01 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
18:00:02 deqp: selector 57
18:00:03 qpa: #beginTestCaseResult dEQP-GLES3.functional.texture.filtering.3d.formats.rgba8_nearest
18:10:00 gc_wdog_arm: feeding bound of 590 s over; WDOG1 resets the SoC within 10000 ms
U-Boot SPL 2020.04-at31 (Nov 01 2025 - 00:00:00 +0000)
DDR Training
Trying to boot from BOOTROM
image offset 0x8000, pagesize 0x200, ivt offset 0x0
NOTICE:  BL31: v2.4(release):2020.04-at28
U-Boot 2020.04-at31 (Nov 01 2025 - 00:00:00 +0000)
Hit any key to stop autoboot:  0
Starting kernel ...
Booting Linux on physical CPU 0x0000000000 [0x410fd034]
Linux version 6.12.105-at (build@host) #1 SMP
18:12:00 Bye!
18:12:01 *** Booting Zephyr OS build v4.1.0 ***
18:12:01 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
18:12:02 deqp: selector 58
18:12:05 deqp: Pass dEQP-GLES3.functional.texture.filtering.3d.formats.rgba8_mipmap
18:12:06 deqp: summary: 1 cases begun, 1 with a result, 0 without
EOF
xout=$(python3 -I "$SCAN" --watch 57,58 "$D/x2-wdog.txt" 2>&1)
xrc=$?
[ "$xrc" = 1 ] && okn "x2 wdog: exit 1" || bad "x2 wdog: exit $xrc"
if grep -qE '^!! NO FINISH row 57 \(watchdog end\) .*case begun, no result: dEQP-GLES3\.functional\.texture\.filtering\.3d\.formats\.rgba8_nearest;' <<<"$xout" &&
    grep -qE '^row +58 .* +4s .*finish' <<<"$xout" && grep -qE '^== .* 6 boot banner' <<<"$xout"; then
    okn "x2 wdog: row 57 watchdog end naming the case, row 58 finishes, 6 banners"
else bad "x2 wdog"; printf '%s\n' "$xout" | sed 's/^/        /'; fi
# the SPL banner lost: the BOOTROM/BL31 lines alone close the row
printf '%s\n' '19:00:00 deqp: selector 20' '19:00:01 qpa: #beginTestCaseResult dEQP-GLES3.functional.shaders.x' \
    'Trying to boot from BOOTROM' 'NOTICE:  BL31: v2.4(release):2020.04-at28' > "$D/x2-lost.txt"
python3 -I "$SCAN" -q "$D/x2-lost.txt" 2>&1 | grep -qE '^!! NO FINISH row 20 \(reset\) .*begun, no result: dEQP-GLES3\.functional\.shaders\.x' &&
    okn "x2 reset with the SPL banner lost: BOOTROM/BL31 line closes the row" || bad "x2 lost banner"
# the image's start inside an open row, no banner at all
printf '%s\n' '19:00:00 deqp: selector 20' '19:00:01 qpa: #beginTestCaseResult dEQP-GLES3.functional.shaders.x' \
    '19:09:00 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s' > "$D/x2-armed.txt"
python3 -I "$SCAN" -q "$D/x2-armed.txt" 2>&1 | grep -qE '^!! NO FINISH row 20 \(reset\)' &&
    okn "armed inside an open row: a reset" || bad "armed inside an open row"

# bad input
python3 -I "$SCAN" "$D/missing.txt" > /dev/null 2>&1
[ "$?" = 2 ] && ok "missing file: exit 2" || bad "missing file"
python3 -I "$SCAN" --watch x "$D/console.txt" > /dev/null 2>&1
[ "$?" = 2 ] && ok "bad --watch: exit 2" || bad "bad --watch"

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
