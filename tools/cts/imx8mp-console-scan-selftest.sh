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

# bad input
python3 -I "$SCAN" "$D/missing.txt" > /dev/null 2>&1
[ "$?" = 2 ] && ok "missing file: exit 2" || bad "missing file"
python3 -I "$SCAN" --watch x "$D/console.txt" > /dev/null 2>&1
[ "$?" = 2 ] && ok "bad --watch: exit 2" || bad "bad --watch"

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
