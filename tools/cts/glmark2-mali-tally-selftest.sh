#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/glmark2-mali-tally-selftest.sh
#
# Host-only check of tools/cts/glmark2-mali-tally.py on the synthetic run
# directories of tools/cts/testdata/glmark2-mali-tally/ (the layout of the
# tally's header; nothing here is a board run):
#   clean/             33 scenes, every one ok, each with a picture
#   faults/            a GPU fault (terrain, panthor MMU fault, black picture),
#                      a hang (refract, timeout=1), a GL error (shadow), a
#                      crash (jellyfish, signal 11, a documentation address
#                      in its log), out of GPU memory, a refused program and
#                      two scenes not run
#   oom/               seven scenes: the back end's OSAL refusal and missing sample
#                      store, GL error 0x505, the kernel's Out of memory (.kmsg) and
#                      page allocation failure (.log) are oom; 0x506 stays gl-error;
#                      a 'draw refused' line is fail with plan #309's note
#   malformed-log/     a log whose result line names another scene
#   malformed-status/  exit=x
#   malformed-kmsg/    a .kmsg without mali-kmsg.py's line
#   runner-dir/        33 scenes in tools/cts/mali-glmark2-run.sh's layout (scenes.tsv,
#                      meta.txt, log/<n>.out|.err, kmsg/<n>.log|.class): ok, an
#                      out-of-gpu-memory, a gl-error, a crash after an exit line
#                      (jellyfish), terrain's crash without one, a gpu-fault; the
#                      runner's verdicts agree with the tally's, and the cases
#                      below change one to see the disagreement (exit 3)
#   pictures.txt       a 'mali' column: the clean run's pictures
# Each output is compared with its *.expected byte for byte (the table without
# its first line, the tool's build); the clean run's picture lines must be the
# fixture table's 'mali' lines and judge as matches with common-tally.py's
# own --gm-pictures. Exits 0 only if every check passes.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TALLY="$HERE/glmark2-mali-tally.py"
T="$HERE/testdata/glmark2-mali-tally"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-mali-tally.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
ok=0
bad=0

check() { # check <label> <command...>: passes when the command exits 0
  local label=$1
  shift
  if "$@"; then
    ok=$((ok + 1))
    echo "  ok    $label"
  else
    bad=$((bad + 1))
    echo "  FAIL  $label"
  fi
}

tally() { python3 -I "$TALLY" "$@"; }

same() { # same <expected file> <command...>: stdout equals the file
  local want=$1
  shift
  "$@" >"$WORK/out" 2>"$WORK/err" || return 1
  cmp -s "$want" "$WORK/out" || { diff -u "$want" "$WORK/out" | head -20; return 1; }
}

table() { tally "$@" | tail -n +2; }

rc_is() { # rc_is <code> <command...>
  local want=$1
  shift
  "$@" >"$WORK/out" 2>"$WORK/err"
  [ "$?" -eq "$want" ]
}

# clean: 33 scenes ok
check "clean: table matches" same "$T/clean.table.expected" table --table "$T/pictures.txt" "$T/clean"
check "clean: 33 ok" grep -q '^scenes run 33 of 33; ok 33;' "$T/clean.table.expected"
check "clean: picture lines match" same "$T/clean.picture-lines.expected" tally --picture-lines "$T/clean"
check "clean: 33 'mali | ' lines" test "$(tally --picture-lines "$T/clean" | grep -c '^mali | ')" -eq 33
check "clean: picture lines are the table's mali column" \
  same <(grep '^mali | ' "$T/pictures.txt") tally --picture-lines "$T/clean"
cat "$T"/clean/*.log >"$WORK/console.txt"
check "clean: common-tally.py --gm-pictures judges every scene a match" \
  rc_is 0 python3 -I "$HERE/common-tally.py" --gm-pictures "$WORK/console.txt" mali --table "$T/pictures.txt"
check "clean: picture lines under --column" \
  test "$(tally --picture-lines --column mali2 "$T/clean" | grep -c '^mali2 | ')" -eq 33
check "clean: results row, 33 of 33, none failed" \
  sh -c "python3 -I '$TALLY' --results-row --table '$T/pictures.txt' '$T/clean' | head -1 | grep -q '33 of 33 scenes ran, 0 failed, 0 GL errors, 0 out of GPU memory'"
check "clean: status row, 33/33" \
  sh -c "python3 -I '$TALLY' --status-row '$T/clean' | grep -q '実行 33/33、正常 33、失敗 0'"
check "clean: no cause" \
  sh -c "python3 -I '$TALLY' --causes '$T/clean' | grep -q 'no failed scene: no issue to open'"
check "clean: without a mali column every picture is no-reference" \
  sh -c "python3 -I '$TALLY' '$T/clean' | grep -q '33 scenes, no-reference 33'"

# faults: each verdict, then the pinned outputs
check "faults: table matches" same "$T/faults.table.expected" table --table "$T/pictures.txt" "$T/faults"
for want in '^23 \[terrain\] <default> +gpu-fault ' '^25 \[refract\] <default> +hang ' \
            '^24 \[shadow\] <default> +gl-error ' '^22 \[jellyfish\] <default> +crash ' \
            '^18 \[buffer\] .*update-method=map +oom ' '^27 \[conditionals\] fragment-steps=5:vertex-steps=0 +fail ' \
            '^33 \[loop\] .* +not-run '; do
  check "faults: /$want/" grep -Eq "$want" "$T/faults.table.expected"
done
check "faults: terrain's picture black" grep -Eq '^23 .* black +3$' "$T/faults.table.expected"
check "faults: results row matches" same "$T/faults.results-row.expected" \
  tally --results-row --table "$T/pictures.txt" "$T/faults"
check "faults: status row matches" same "$T/faults.status-row.expected" \
  tally --status-row --table "$T/pictures.txt" "$T/faults"
check "faults: causes match" same "$T/faults.causes.expected" tally --causes "$T/faults"
check "faults: 6 causes" grep -q '): 6$' "$T/faults.causes.expected"
check "faults: the refusal is plan #309's" grep -q 'plan #309 owns these' "$T/faults.causes.expected"
check "faults: no address, login or UART in any output" \
  sh -c "! cat '$T'/faults.*.expected | grep -Eq '192\.0\.2\.|root@|/dev/tty'"
check "faults: the log does hold the address" grep -q '192\.0\.2\.7' "$T"/faults/22-jellyfish-default.log
check "faults: picture lines skip the scenes without a value" \
  test "$(tally --picture-lines "$T/faults" 2>/dev/null | grep -c '^mali | ')" -eq 28

# oom: the memory failures of the back end, GL_OUT_OF_MEMORY and the kernel
check "oom: table matches" same "$T/oom.table.expected" table --table "$T/pictures.txt" "$T/oom"
check "oom: causes match" same "$T/oom.causes.expected" tally --causes "$T/oom"
for want in '^01 .* oom ' '^02 .* oom ' '^03 .* oom ' '^04 .* oom ' '^05 .* oom ' \
            '^06 .* gl-error ' '^07 .* fail '; do
  check "oom: /$want/" grep -Eq "$want" "$T/oom.table.expected"
done
check "oom: the OSAL refusal, the missing sample store, GL 0x505 and both kernel lines are oom" \
  grep -q 'oom 5, gl-error 1, fail 1;' "$T/oom.table.expected"
check "oom: the .kmsg line is the cause, its time stamp off" \
  grep -q 'oom: Out of memory: Killed process # ' "$T/oom.causes.expected"
check "oom: the .kmsg line is in the excerpt" \
  grep -q '^  > kmsg: Out of memory: Killed process 2841' "$T/oom.causes.expected"
check "oom: the .log kernel line's time stamp is off the cause" \
  grep -q 'oom: glmark2-es2-mal: page allocation failure' "$T/oom.causes.expected"
check "oom: GL 0x505 is no gl-error cause" \
  sh -c "! grep -q 'gl-error: .*0x505' '$T/oom.causes.expected'"
check "oom: GL 0x506 stays gl-error" grep -q 'gl-error: GL error 0x506 in glDrawArrays' "$T/oom.causes.expected"
check "oom: the OSAL refusal is not plan #309's" \
  sh -c "! awk -v n=1 '/^## Draft issue /{on = (\$4 == n)} on' '$T/oom.causes.expected' | grep -q 'plan #309'"
check "oom: the missing sample store is not plan #309's" \
  sh -c "! awk -v n=2 '/^## Draft issue /{on = (\$4 == n)} on' '$T/oom.causes.expected' | grep -q 'plan #309'"
check "oom: the draw refused line is still plan #309's" \
  sh -c "awk -v n=7 '/^## Draft issue /{on = (\$4 == n)} on' '$T/oom.causes.expected' | grep -q 'plan #309 owns these'"

# runner-dir: a mali-glmark2-run.sh run directory, read directly
R="$T/runner-dir"
check "runner-dir: table matches" same "$T/runner-dir.table.expected" table --table "$T/pictures.txt" "$R"
check "runner-dir: picture lines match" same "$T/runner-dir.picture-lines.expected" tally --picture-lines "$R"
check "runner-dir: results row matches" same "$T/runner-dir.results-row.expected" \
  tally --results-row --table "$T/pictures.txt" "$R"
check "runner-dir: the runner's verdicts agree, exit 0" rc_is 0 tally "$R"
check "runner-dir: no disagreement on stderr" test ! -s "$WORK/err"
for want in '^18 .* oom ' '^22 .* crash ' '^23 .* crash ' '^24 .* gl-error ' '^25 .* gpu-fault '; do
  check "runner-dir: /$want/" grep -Eq "$want" "$T/runner-dir.table.expected"
done
check "runner-dir: gpu-fault, crash x2, oom, gl-error, 28 ok" \
  grep -q 'gpu-fault 1, crash 2, oom 1, gl-error 1, ok 28;' "$T/runner-dir.table.expected"
check "runner-dir: the kmsg line is counted" grep -Eq '^25 .* 3$' "$T/runner-dir.table.expected"
check "runner-dir: a crash after the exit line has no signal number invented" \
  grep -q 'crash: killed by a signal (number not recorded)' "$T/runner-dir.table.expected"
check "runner-dir: terrain's crash without an exit line is found by the tally" \
  grep -q 'crash: no yakogl-mali-exit line' "$T/runner-dir.table.expected"
check "runner-dir: 31 picture lines" test "$(tally --picture-lines "$R" 2>/dev/null | grep -c '^mali | ')" -eq 31
check "runner-dir: the run's own pictures.txt is the picture lines" \
  same <(grep '^mali | ' "$R/pictures.txt") tally --picture-lines "$R"
cp -r "$R" "$WORK/mismatch"
awk -F'\t' -v OFS='\t' 'NR == 20 { $3 = "gl-error" } { print }' "$R/scenes.tsv" >"$WORK/mismatch/scenes.tsv"
check "runner-dir: a differing verdict exits 3" rc_is 3 tally "$WORK/mismatch"
check "runner-dir: the scene and both verdicts on stderr" \
  grep -q 'scene \[buffer\] .*update-method=subdata: runner verdict gl-error, tally verdict ok' "$WORK/err"
check "runner-dir: the table is still printed" grep -q '^scenes run 33 of 33' "$WORK/out"
check "runner-dir: --allow-verdict-mismatch exits 0" rc_is 0 tally --allow-verdict-mismatch "$WORK/mismatch"
check "runner-dir: ... and still names it" \
  grep -q '^glmark2-mali-tally: allowed: scene .*runner verdict gl-error' "$WORK/err"
cp -r "$R" "$WORK/notrun"
awk -F'\t' -v OFS='\t' 'NR == 34 { $3 = "not-run"; $4 = $5 = $6 = $7 = "-" } { print }' "$R/scenes.tsv" >"$WORK/notrun/scenes.tsv"
check "runner-dir: a not-run scene gives no disagreement" rc_is 0 tally "$WORK/notrun"
check "runner-dir: ... and the table says not-run" grep -Eq '^33 .* not-run ' "$WORK/out"
cp -r "$R" "$WORK/badrow"
sed -i '2s/^build:use-vbo=false/nosuch:scene/' "$WORK/badrow/scenes.tsv"
check "runner-dir: an unknown scene exits 1" rc_is 1 tally "$WORK/badrow"

# malformed input exits 1, usage 2
for m in log status kmsg; do
  check "malformed-$m exits 1" rc_is 1 tally "$T/malformed-$m"
  check "malformed-$m names it" grep -q 'malformed: ' "$WORK/err"
done
check "missing run directory exits 1" rc_is 1 tally "$T/nope"
check "no run directory: usage, exit 2" rc_is 2 tally
check "unknown option: usage, exit 2" rc_is 2 tally --nope "$T/clean"

echo "glmark2-mali-tally-selftest: $ok passed, $bad failed"
[ "$bad" -eq 0 ]
