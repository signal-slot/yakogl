#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_deqp_sweep_selftest.sh
#
# Host-only check of tools/cts/mp2-deqp-sweep.sh: a synthetic deqp image (one
# case slot, built as mp2_deqp_slot_selftest.sh builds one) and a fake runner
# under $TMPDIR that reads the slot's filter from the image it is given and
# prints "deqp: Pass <case>" for each case. Checks the dry run (every command
# printed, nothing made or run), the sweep (shard files, one slot image per
# shard, one log per shard, the final diff call), the resume skip (a shard
# whose log holds a result is not run again, a damaged one is), the shard
# range, the default case list with its exclusions, and the refusals; the
# every runner exit code (0 2 3 4 5 6 7 8 129 130 143, and an unknown one stops); the
# stop on runner exit 5 and 7 and on SIGINT/SIGTERM (undo lines shown, no next
# shard, no diff, exit 3 / 130 / 143) and the RUN_BOUND default (lifetime + 100 s).
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
SWEEP=$HERE/mp2-deqp-sweep.sh
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-deqp-sweep.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

# a synthetic image: random bytes around one slot (room 4096)
python3 -I - "$D/zephyr.bin" <<'PY'
import os, struct, sys
slot = b"yakogl-deqp-case-slot/1\0" + struct.pack("<IiI", 4096, -1, 0) + bytes(4096)
open(sys.argv[1], "wb").write(os.urandom(0x800) + slot + os.urandom(1024))
PY
# the fake runner: run <bin> <logdir> <name>; logs its calls, prints one Pass per case of the slot
cat >"$D/runner.sh" <<'EOS'
#!/bin/bash
[ "$1" = run ] || exit 2
echo "$2 $3 $4" >> "$FAKE_CALLS"
mkdir -p "$3/stamp-$4"
python3 -I - "$2" <<'PY' | tr ',' '\n' | sed 's/^/deqp: Pass /' > "$3/stamp-$4/console.txt"
import sys
d = open(sys.argv[1], "rb").read()
at = d.find(b"yakogl-deqp-case-slot/1\0") + 36
print(d[at:d.index(b"\0", at)].decode())
PY
echo "fake runner: $4 done"
EOS
chmod +x "$D/runner.sh"
export FAKE_CALLS=$D/calls.txt
export MP2_SWEEP_RUNNER=$D/runner.sh

# six real mustpass cases, a budget of two cases per shard (100 ms each by default)
grep -v '^#' "$HERE/gles2-mustpass.txt" | head -6 > "$D/cases.txt"
sw() { bash "$SWEEP" --report-only --case-list "$D/cases.txt" --budget-ms 250 "$@"; }

echo "dry run"
out=$(sw --dry-run "$D/zephyr.bin" "$D/o1" 2>&1); rc=$?
chk "dry: exit 0" test $rc -eq 0
chk "dry: shard command" grep -q 'mp2-deqp-shards.py .*cases.txt --out .*o1/shards --budget-ms 250' <<<"$out"
chk "dry: slot-write command" grep -q 'mp2-deqp-slot.py write .*zephyr.bin --case-list .*shard-NNNN.txt -o .*slot-NNNN.bin' <<<"$out"
chk "dry: runner command" grep -q "$D/runner.sh run .*slot-NNNN.bin .*shard-NNNN shard-NNNN" <<<"$out"
chk "dry: diff command" grep -q 'mp2-p9-diff.py .*o1/shard-\*.log' <<<"$out"
chk "dry: nothing made" test ! -e "$D/o1"
chk "dry: runner not called" test ! -e "$D/calls.txt"

echo "sweep"
out=$(sw "$D/zephyr.bin" "$D/o2" 2>&1); rc=$?
chk "sweep: exit 0" test $rc -eq 0
chk "sweep: 3 shards" test "$(ls "$D"/o2/shards/shard-*.txt | wc -l)" -eq 3
chk "sweep: 3 slot images" test "$(ls "$D"/o2/slot-*.bin | wc -l)" -eq 3
chk "sweep: 3 runner calls" test "$(wc -l < "$D/calls.txt")" -eq 3
chk "sweep: the runner got the slot image and name" grep -q "$D/o2/slot-0002.bin $D/o2/run/shard-0002 shard-0002" "$D/calls.txt"
chk "sweep: each log holds the runner output" grep -q 'fake runner: shard-0002 done' "$D/o2/shard-0002.log"
chk "sweep: each log holds the console" grep -qx "deqp: Pass $(sed -n 3p "$D/cases.txt")" "$D/o2/shard-0001.log"
chk "sweep: the image is the source plus the slot" test "$(stat -c %s "$D/o2/slot-0001.bin")" -eq "$(stat -c %s "$D/zephyr.bin")"
chk "sweep: source image untouched" test "$(python3 -I "$HERE/mp2-deqp-slot.py" show "$D/zephyr.bin" | grep -c 'slot not written')" -eq 1
chk "sweep: diff ran" grep -q '^mp2-p9-diff: shared ' <<<"$out"
chk "sweep: summary" grep -q 'mp2-deqp-sweep: 3 run, 0 skipped, 0 failed' <<<"$out"

echo "resume"
: > "$D/calls.txt"
printf 'garbage, no result\n' > "$D/o2/shard-0002.log"
out=$(sw "$D/zephyr.bin" "$D/o2" 2>&1); rc=$?
chk "resume: exit 0" test $rc -eq 0
chk "resume: only the damaged shard ran" test "$(wc -l < "$D/calls.txt")" -eq 1
chk "resume: that is shard 2" grep -q 'shard-0002$' "$D/calls.txt"
chk "resume: summary" grep -q 'mp2-deqp-sweep: 1 run, 2 skipped, 0 failed' <<<"$out"
: > "$D/calls.txt"
out=$(sw "$D/zephyr.bin" "$D/o2" 2>&1)
chk "resume: all done, nothing run" test ! -s "$D/calls.txt"
chk "resume: diff still called" grep -q '^mp2-p9-diff: ' <<<"$out"

# patch A: a log that stops part way (a lifetime end) is not a done shard
: > "$D/calls.txt"
printf 'deqp: Pass %s\n' "$(sed -n 3p "$D/cases.txt")" > "$D/o2/shard-0001.log"   # shard 1 is cases 3 and 4
out=$(sw "$D/zephyr.bin" "$D/o2" 2>&1)
chk "partial log: the shard runs again" grep -q 'shard-0001$' "$D/calls.txt"
chk "partial log: nothing else runs" test "$(wc -l < "$D/calls.txt")" -eq 1
chk "partial log: rerun log holds both cases" test "$(grep -c '^deqp: Pass' "$D/o2/shard-0001.log")" -eq 2
: > "$D/calls.txt"
printf 'x 10:00:01 deqp: Pass %s\r\n10:00:02 deqp: Fail %s\r\n' "$(sed -n 3p "$D/cases.txt")" "$(sed -n 4p "$D/cases.txt")" > "$D/o2/shard-0001.log"
out=$(sw "$D/zephyr.bin" "$D/o2" 2>&1)
chk "stamped CRLF log with every case: skipped" test ! -s "$D/calls.txt"

# patch C: the diff gets --cases, so cases outside the sweep are not missing
echo "cases for the diff"
out=$(sw --dry-run "$D/zephyr.bin" "$D/o1" 2>&1)
chk "dry: diff gets --cases OUT/cases.txt" grep -q "mp2-p9-diff.py .*--cases $D/o1/cases.txt " <<<"$out"
: > "$D/calls.txt"
out=$(sw --first 1 --last 1 "$D/zephyr.bin" "$D/o8" 2>&1)
chk "one shard of three: only the sweep's 4 other cases are missing" grep -q '^mp2-p9-diff: .*missing 4$' <<<"$out"

# patch B: the shard split gets the image's slot room as --max-bytes
python3 -I - "$D/small.bin" <<'PY'
import os, struct, sys
slot = b"yakogl-deqp-case-slot/1\0" + struct.pack("<IiI", 111, -1, 0) + bytes(111)
open(sys.argv[1], "wb").write(os.urandom(0x800) + slot + os.urandom(1024))
PY
out=$(sw --dry-run "$D/small.bin" "$D/o9" 2>&1)
chk "dry: --max-bytes is the slot room (110)" grep -q 'mp2-deqp-shards.py .*--max-bytes 110$' <<<"$out"
out=$(bash "$SWEEP" --report-only --case-list "$D/cases.txt" --budget-ms 100000 "$D/small.bin" "$D/o9" 2>&1)
chk "small slot: 6 cases in 3 shards (2 fit 110 bytes, 3 do not)" test "$(ls "$D"/o9/shards/shard-*.txt | wc -l)" -eq 3
out=$(bash "$SWEEP" --report-only --case-list "$D/cases.txt" --budget-ms 100000 "$D/zephyr.bin" "$D/o10" 2>&1)
chk "room 4096: one shard" test "$(ls "$D"/o10/shards/shard-*.txt | wc -l)" -eq 1

echo "range"
: > "$D/calls.txt"
out=$(sw --first 2 --last 2 "$D/zephyr.bin" "$D/o3" 2>&1); rc=$?
chk "range: only shard 2 ran" test "$(wc -l < "$D/calls.txt")" -eq 1
chk "range: no log for 1" test ! -e "$D/o3/shard-0001.log"

echo "exclude and redaction"
sed -n 1p "$D/cases.txt" > "$D/excl.txt"
out=$(sw --dry-run --exclude "$D/excl.txt" "$D/zephyr.bin" "$D/o6" 2>&1)
chk "excl dry: diff gets --exclude" grep -q "mp2-p9-diff.py .*--exclude $D/excl.txt .*o6/shard-" <<<"$out"
chk "excl dry: redaction check printed" grep -q 'mp2-log-redact-check.sh .*o6' <<<"$out"
out=$(sw --exclude "$D/excl.txt" "$D/zephyr.bin" "$D/o6" 2>&1); rc=$?
chk "excl: exit 0" test $rc -eq 0
chk "excl: summary says excluded 1" grep -q '^mp2-p9-diff: .*excluded 1' <<<"$out"
out=$(MP2_DEQP_EXCLUDE=$D/excl.txt sw "$D/zephyr.bin" "$D/o7" 2>&1)
chk "excl env: MP2_DEQP_EXCLUDE reaches the diff" grep -q '^mp2-p9-diff: .*excluded 1' <<<"$out"
out=$(sw "$D/zephyr.bin" "$D/o2" 2>&1)
chk "no exclude: no 'excluded' in the summary" bash -c '! grep -q "^mp2-p9-diff: .*excluded" <<<"$1"' _ "$(grep '^mp2-p9-diff: ' <<<"$out")"
chk "clean logs: redaction check clean" grep -q 'clean (' <<<"$out"
echo "uart on /dev/ttyUSB0 at 198.51.100.7" >> "$D/o6/shard-0001.log"
out=$(sw --exclude "$D/excl.txt" "$D/zephyr.bin" "$D/o6" 2>&1); rc=$?
chk "redaction hit: exit 1" test $rc -eq 1
chk "redaction hit: says to redact" grep -q 'must be redacted before commit' <<<"$out"

echo "failures"
printf "#!/bin/bash\nexit 4\n" > "$D/rc4.sh"; chmod +x "$D/rc4.sh"
out=$(MP2_SWEEP_RUNNER=$D/rc4.sh sw "$D/zephyr.bin" "$D/o4" 2>&1); rc=$?
chk "runner failure: exit 1" test $rc -eq 1
chk "runner failure: counted" grep -q '3 run, 0 skipped, 3 failed, 0 lifetime' <<<"$out"
out=$(bash "$SWEEP" --dry-run 2>&1); rc=$?
chk "usage: exit 2" test $rc -eq 2
out=$(sw "$D/nothere.bin" "$D/o5" 2>&1); rc=$?
chk "no image: exit 2" test $rc -eq 2

echo "stop: exit 5 and 7"
cat >"$D/stop.sh" <<'EOS'
#!/bin/bash
echo "$2 $3 $4" >> "$FAKE_CALLS"
echo "board-stm32mp2.sh: WARNING: left behind: the board was rebooted and is not back"
echo "board-stm32mp2.sh: the undo, once the board answers: tools/board-stm32mp2.sh disarm (record: x)"
exit "$STOP_RC"
EOS
chmod +x "$D/stop.sh"
for rc in 5 7; do
    : > "$D/calls.txt"
    out=$(STOP_RC=$rc MP2_SWEEP_RUNNER=$D/stop.sh sw "$D/zephyr.bin" "$D/os$rc" 2>&1); r=$?
    chk "exit $rc: sweep exit 3" test $r -eq 3
    chk "exit $rc: only the first shard ran" test "$(wc -l < "$D/calls.txt")" -eq 1
    chk "exit $rc: undo line printed" grep -q 'runner: .*tools/board-stm32mp2.sh disarm' <<<"$out"
    chk "exit $rc: says a person is needed" grep -q 'board needs a person' <<<"$out"
    chk "exit $rc: no diff" bash -c '! grep -q "^mp2-p9-diff" <<<"$1"' _ "$out"
done
# every rc of the runner's header table: stop (exit) or go on to the next shard (3 shards)
for rc in 0 2 3 4 5 6 7 8 129 130 143 1 9 42; do
    : > "$D/calls.txt"
    out=$(STOP_RC=$rc MP2_SWEEP_RUNNER=$D/stop.sh sw "$D/zephyr.bin" "$D/ol$rc" 2>&1); r=$?
    case $rc in
    2) want_r=2 want_calls=1 ;;
    5|7|129|130|143|1|9|42) want_r=3 want_calls=1 ;;
    *) want_r=1 want_calls=3 ;;
    esac
    case $rc in 0|8) want_r=$(( r == 0 || r == 1 ? r : 99 )) ;; esac
    chk "rc $rc: sweep exit $want_r" test $r -eq $want_r
    chk "rc $rc: $want_calls shard(s) ran" test "$(wc -l < "$D/calls.txt")" -eq $want_calls
    case $rc in
    0) ;;
    8) chk "rc 8: says lifetime" grep -q 'runner exit 8 (verdict=lifetime' <<<"$out"
       chk "rc 8: counted apart from failures" grep -q ' 0 failed, 3 lifetime' <<<"$out" ;;
    5|7|2|129|130|143) chk "rc $rc: STOPPED" grep -q 'STOPPED: shard [0-9]*: runner exit '"$rc"' ' <<<"$out" ;;
    1|9|42) chk "rc $rc: unknown code stops with a message" grep -q "STOPPED: shard [0-9]*: runner exit $rc is not a code the runner documents" <<<"$out"
            chk "rc $rc: no diff" bash -c '! grep -q "^mp2-p9-diff" <<<"$1"' _ "$out" ;;
    *) chk "rc $rc: named, goes on" grep -q "shard [0-9]*: runner exit $rc (" <<<"$out" ;;
    esac
done

echo "stop: SIGINT and SIGTERM"
cat >"$D/slow.sh" <<'EOS'
#!/bin/bash
echo "$2 $3 $4" >> "$FAKE_CALLS"
trap 'echo "board-stm32mp2.sh: SIGINT: ending through the undo"; echo "board-stm32mp2.sh: the undo, once the board answers: tools/board-stm32mp2.sh disarm"; exit 130' INT TERM
echo "fake runner started"
while :; do sleep 0.1; done
EOS
chmod +x "$D/slow.sh"
for sg in INT TERM; do
    : > "$D/calls.txt"
    ( set -m
      MP2_SWEEP_RUNNER=$D/slow.sh python3 -I -c 'import os, signal, sys
# the gate may run this self-test with SIGINT ignored (a background step): an
# ignored signal is inherited and cannot be trapped, so restore the defaults
for s in (signal.SIGINT, signal.SIGTERM): signal.signal(s, signal.SIG_DFL)
os.execvp("bash", ["bash"] + sys.argv[1:])' "$SWEEP" --report-only --case-list "$D/cases.txt" --budget-ms 250 "$D/zephyr.bin" "$D/o$sg" > "$D/out$sg.txt" 2>&1 &
      p=$!
      for _ in $(seq 100); do grep -q 'fake runner started' "$D/o$sg/shard-0001.log" 2>/dev/null && break; sleep 0.1; done
      kill -s $sg $p
      wait $p; echo $? > "$D/rc$sg.txt" )
    want=130; [ $sg = TERM ] && want=143
    chk "SIG$sg: exit $want" test "$(cat "$D/rc$sg.txt")" -eq $want
    chk "SIG$sg: only the first shard ran" test "$(wc -l < "$D/calls.txt")" -eq 1
    chk "SIG$sg: runner's undo line shown" grep -q 'runner: .*the undo' "$D/out$sg.txt"
    chk "SIG$sg: says a person is needed" grep -q 'board needs a person' "$D/out$sg.txt"
    chk "SIG$sg: no diff" bash -c '! grep -q "^mp2-p9-diff" "$1"' _ "$D/out$sg.txt"
done

echo "RUN_BOUND"
cat >"$D/rb.sh" <<'EOS'
#!/bin/bash
echo "RB=${RUN_BOUND-unset}" >> "$FAKE_CALLS"
EOS
chmod +x "$D/rb.sh"
: > "$D/calls.txt"
life=$(sed -n 's/^CONFIG_YAKOGL_STM32MP2_LIFETIME_S=//p' "$HERE/../../driver/samples/zephyr/deqp/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf")
out=$(env -u RUN_BOUND MP2_SWEEP_RUNNER=$D/rb.sh bash "$SWEEP" --report-only --case-list "$D/cases.txt" --first 1 --last 1 --budget-ms 250 "$D/zephyr.bin" "$D/orb1" 2>&1)
chk "unset: lifetime $life + 100 given to the runner" grep -qx "RB=$((life + 100))" "$D/calls.txt"
chk "unset: the value is printed" grep -q "RUN_BOUND=$((life + 100)) " <<<"$out"
: > "$D/calls.txt"
out=$(RUN_BOUND=77 MP2_SWEEP_RUNNER=$D/rb.sh bash "$SWEEP" --report-only --case-list "$D/cases.txt" --first 1 --last 1 --budget-ms 250 "$D/zephyr.bin" "$D/orb2" 2>&1)
chk "set: the caller's value kept" grep -qx 'RB=77' "$D/calls.txt"
out=$(env -u RUN_BOUND bash "$SWEEP" --dry-run "$D/zephyr.bin" "$D/orb3" 2>&1)
chk "dry run prints RUN_BOUND" grep -q "RUN_BOUND=$((life + 100)) " <<<"$out"

echo "default list"
out=$(bash "$SWEEP" --dry-run "$D/zephyr.bin" "$D/o6" 2>&1)
n=$(sed -n 's/^mp2-deqp-sweep: dry run: \([0-9]*\) cases.*/\1/p' <<<"$out")
want=$(sh "$HERE/mp2-deqp-list-check.sh" | sed -n 's/.*, \([0-9]*\) mustpass cases to run/\1/p')
chk "default: $n cases equal list-check's $want" test -n "$n" -a "$n" = "$want"

echo "$pass ok, $fail failed"
[ "$fail" -eq 0 ]
