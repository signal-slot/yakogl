#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-scene-table-selftest.sh
#
# Host-only checks of tools/cts/x2-scene-table.py against synthetic record
# fixtures written at run time in $TMPDIR: a green run, a knob-on run, a knob
# built off, a stop run (no verdict line) and a run whose files carry an address.
# No board, no network, nothing outside the scratch directory is touched.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=$HERE/x2-scene-table.py
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check()  { case "$3" in *"$2"*) ok "$1" ;; *) bad "$1: wanted '$2' in: $3" ;; esac; }
absent() { case "$3" in *"$2"*) bad "$1: found '$2' in: $3" ;; *) ok "$1" ;; esac; }
rc_is()  { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: rc $3, wanted $2"; fi; }
row_n() { grep -cE '^\| [0-9a-f]{8} \|' <<<"$1"; }

command -v python3 > /dev/null || { echo "x2-scene-table-selftest: python3 missing from the host" >&2; exit 2; }

T=$(mktemp -d "${TMPDIR:-/tmp}/x2-scene-table-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT

# ---- the fixtures: record directories as tools/board.sh run writes them -------------------
R=$T/rec
mkdir -p "$R/empty"

# terrain, a green run without the knob: the console names the scene with a header line
# and a bare result line (driver lines in between), as on a real X2 console.
D=$R/terrain/20261113-1000-terrain-nosel-72972ba4
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=terrain-nosel
started=2026-11-13 10:00:00
ended=2026-11-13 10:01:10
EOF
cat > "$D/console.txt" <<'EOF'
10:00:05 gc_wdog_arm: armed (the watchdog marker line)
10:00:07 glmark2: score lines follow after the scene's frames
10:00:20 [terrain] <default>:
10:00:30 glmark2: PERF page 0 asked (the frame-breakdown record of the last scene)
10:00:31 link noise 198.51.100.9 read off the console
10:00:40 glmark2-frame: [terrain] <default>: 61 frames; us per frame: gl 240000, swap 8000, show 1, other 2000, frame 250000
10:00:41  FPS: 4 FrameTime: 250.000 ms
10:00:42 glmark2: score 4: 1 scenes ran, 0 failed, 0 unsupported (1 list entry)
EOF
cat > "$D/INDEX.txt" <<'EOF'
# tools/board.sh run record (driver/docs/one-method.md section 4; records/README.md)
runner: tools/board.sh build dev
board: x2
boot: kexec-dev (DEV: a development run, never a row)
name: terrain-nosel
started: 2026-11-13 10:00:00
ended: 2026-11-13 10:01:10
verdict: board.sh x2 terrain-nosel rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s
records:
  none (the console has no yk-record line: the image prints none)
files (sha256, lines):
  0000000000000000000000000000000000000000000000000000000000000000  8  console.txt
EOF

# shadow, the knob on: the console prints the sample's gc_tiled_rt line, the result line
# carries FPS and FrameTime, the record words decode to a page 0 the console already beats.
D=$R/shadow-tiled_rt/20261113-1100-shadow-p0-1234abcd
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=shadow-p0
EOF
cat > "$D/console.txt" <<'EOF'
11:00:05 glmark2: gc_tiled_rt=1 (tiled render targets; a measurement, not a score)
11:00:20 [shadow] <default>: FPS: 10 FrameTime: 100.000 ms
EOF
cat > "$D/INDEX.txt" <<'EOF'
verdict: board.sh x2 shadow-p0 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:gm_page resets=0 record=594b0c00,09c40064,00000000 back_after=60s
records:
  experiment 0 page 0: 100 90000 2000 0 n/a n/a
EOF

# refract, the knob on by the -tiled_rt directory but with no verdict line: a stop row.
# Its console still carries a whole result line, so the numbers print with the stop.
D=$R/refract-tiled_rt/20261113-1200-refract-nosel-abcd0000
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=refract-nosel
EOF
cat > "$D/console.txt" <<'EOF'
12:00:20 [refract] <default>: FPS: 2 FrameTime: 500.000 ms
EOF
cat > "$D/INDEX.txt" <<'EOF'
board: x2
name: refract-nosel
started: 2026-11-13 12:00:00
records:
  none (the console has no yk-record line: the image prints none)
EOF

# refract, no knob, a verdict line carrying an address and a UART path: both leave redacted.
D=$R/refract/20261113-1300-refract-nosel-ee00ee00
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=refract-nosel
EOF
cat > "$D/console.txt" <<'EOF'
13:00:20 [refract] <default>: FPS: 3 FrameTime: 333.333 ms
EOF
cat > "$D/INDEX.txt" <<'EOF'
verdict: board.sh x2 refract-nosel rc=4 verdict=failed boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s host 198.51.100.9 on /dev/ttyAMA0
EOF

# shadow, the knob built explicitly off (-tiled_rt_n): a whole result line without FrameTime:
# ms a frame comes from 1000 / FPS.
D=$R/shadow-tiled_rt_n/20261113-1400-shadow-nosel-0f0f0f0f
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=shadow-nosel
EOF
cat > "$D/console.txt" <<'EOF'
14:00:20 [shadow] <default>: FPS: 5
EOF
cat > "$D/INDEX.txt" <<'EOF'
verdict: board.sh x2 shadow-nosel rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s
EOF

# shadow, a run that died before its INDEX.txt was written (no INDEX at all): a stop row.
D=$R/shadow-tiled_rt/20261113-1600-shadow-p1-c0ffee00
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=shadow-p1
EOF

# shadow, no knob, the bare result line named only by the header line: 1000 / FPS again.
D=$R/shadow/20261113-1450-shadow-nosel-9999aaaa
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=shadow-nosel
EOF
cat > "$D/console.txt" <<'EOF'
14:50:10 [shadow] <default>:
14:50:11 glmark2: a driver line between the scene's start and its end
14:50:20  FPS: 8
EOF
cat > "$D/INDEX.txt" <<'EOF'
verdict: board.sh x2 shadow-nosel rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s
EOF

# terrain, the knob on, no result line at all: ms a frame from the record page 0 word.
D=$R/terrain-tiled_rt/20261113-1500-terrain-p0-deadbeef
mkdir -p "$D"
cat > "$D/meta.txt" <<'EOF'
boot=kexec-dev
name=terrain-p0
EOF
cat > "$D/console.txt" <<'EOF'
15:00:05 glmark2: gc_tiled_rt=1 (tiled render targets; a measurement, not a score)
15:00:30 glmark2: the board did not print the scene's result line
EOF
cat > "$D/INDEX.txt" <<'EOF'
verdict: board.sh x2 terrain-p0 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:gm_page resets=0 record=594b0c00,00015f90,000007d0 back_after=60s
records:
  experiment 0 page 0: 100 90000 2000 0 n/a n/a
EOF

run() { out=$(python3 "$SCRIPT" "$@" 2>&1); rc=$?; }

# ---- the whole table from one container ----------------------------------------------------
run "$R"
rc_is "container run: exit 0" 0 "$rc"
check "header row" "| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line |" "$out"
n=$(row_n "$out")
[ "$n" = 8 ] && ok "eight rows, one a record directory" || bad "rows: $n, wanted 8: $out"

check "green run, FrameTime a row (bare result line named by its header)" \
  "| 72972ba4 | absent | terrain | 250.000 | 4 | board.sh x2 terrain-nosel rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s |" "$out"
check "knob on from the console line, FrameTime beats the page 0 record" \
  "| 1234abcd | on | shadow | 100.000 | 10 | board.sh x2 shadow-p0 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:gm_page resets=0 record=594b0c00,09c40064,00000000 back_after=60s |" "$out"
check "knob on from the -tiled_rt directory (no console knob line)" \
  "| abcd0000 | on | refract | 500.000 | 2 | stop: no verdict line in INDEX.txt |" "$out"
check "the verdict line leaves redacted: IPv4 becomes 192.0.2.n, the UART path becomes <uart-path>" \
  "| ee00ee00 | absent | refract | 333.333 | 3 | board.sh x2 refract-nosel rc=4 verdict=failed boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s host 192.0.2.n on <uart-path> |" "$out"
absent "no IPv4-like string printed" "198.51.100" "$out"
absent "no UART path printed" "ttyAMA0" "$out"
check "knob off from the -tiled_rt_n directory, ms from 1000 / FPS (5 -> 200.000)" \
  "| 0f0f0f0f | off | shadow | 200.000 | 5 | board.sh x2 shadow-nosel rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=none resets=0 record=absent back_after=60s |" "$out"
check "stop row for the run with no INDEX.txt at all" \
  "| c0ffee00 | on | shadow | - | - | stop: no INDEX.txt |" "$out"
check "bare FPS line named by the header line, ms from 1000 / FPS (8 -> 125.000)" \
  "| 9999aaaa | absent | shadow | 125.000 | 8 |" "$out"
check "no result line: ms a frame from the record page 0 word v1 (90000 us)" \
  "| deadbeef | on | terrain | 90.000 | - | board.sh x2 terrain-p0 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:gm_page resets=0 record=594b0c00,00015f90,000007d0 back_after=60s |" "$out"
check "the note below the table names the stop rows" \
  "x2-scene-table: stop: 2 record(s) printed no verdict line: refract-nosel, shadow-p1" "$out"

p1=$(grep -n '72972ba4' <<<"$out" | head -1 | cut -d: -f1)
p2=$(grep -n 'deadbeef' <<<"$out" | head -1 | cut -d: -f1)
[ "$p1" -lt "$p2" ] && ok "rows grouped by scene, commit order inside" || bad "scene grouping: 72972ba4 at $p1, deadbeef at $p2"
p1=$(grep -n '1234abcd' <<<"$out" | head -1 | cut -d: -f1)
p2=$(grep -n '9999aaaa' <<<"$out" | head -1 | cut -d: -f1)
[ "$p1" -lt "$p2" ] && ok "the knob-on shadow row sits beside the knob-off one" || bad "shadow grouping: 1234abcd at $p1, 9999aaaa at $p2"

# ---- a record directory given directly, and a container plus the same record ----------------
run "$R/terrain/20261113-1000-terrain-nosel-72972ba4" "$R/refract/20261113-1300-refract-nosel-ee00ee00"
rc_is "record directories given directly: exit 0" 0 "$rc"
n=$(row_n "$out")
[ "$n" = 2 ] && ok "two rows for two records" || bad "rows: $n, wanted 2"
check "direct record: the scene from the run name beside the console" "| 72972ba4 | absent | terrain | 250.000 | 4 |" "$out"

run "$R" "$R/terrain/20261113-1000-terrain-nosel-72972ba4"
n=$(row_n "$out")
[ "$n" = 8 ] && ok "a record named beside its container is printed once" || bad "dedup: $n rows, wanted 8"

# ---- the refusals ------------------------------------------------------------------------------
run
rc_is "no argument: usage (exit 2)" 2 "$rc"
check "usage names the script" "tools/cts/x2-scene-table.py" "$out"
run --bogus "$R"
rc_is "unknown option: refused (exit 2)" 2 "$rc"
run "$R/no-such-dir"
rc_is "a path that is not a directory: refused (exit 2)" 2 "$rc"
run "$R/empty"
rc_is "no record directories: exit 1" 1 "$rc"
check "the empty-container message says what it looked for" "no record directories" "$out"

echo
if [ "$fail" = 0 ]; then echo "x2-scene-table-selftest: all ok"; else echo "x2-scene-table-selftest: $fail failed"; exit 1; fi
