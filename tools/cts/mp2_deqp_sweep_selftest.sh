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
# range, the default case list with its exclusions, and the refusals.
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

echo "range"
: > "$D/calls.txt"
out=$(sw --first 2 --last 2 "$D/zephyr.bin" "$D/o3" 2>&1); rc=$?
chk "range: only shard 2 ran" test "$(wc -l < "$D/calls.txt")" -eq 1
chk "range: no log for 1" test ! -e "$D/o3/shard-0001.log"

echo "failures"
out=$(MP2_SWEEP_RUNNER=false sw "$D/zephyr.bin" "$D/o4" 2>&1); rc=$?
chk "runner failure: exit 1" test $rc -eq 1
chk "runner failure: counted" grep -q '3 run, 0 skipped, 3 failed' <<<"$out"
out=$(bash "$SWEEP" --dry-run 2>&1); rc=$?
chk "usage: exit 2" test $rc -eq 2
out=$(sw "$D/nothere.bin" "$D/o5" 2>&1); rc=$?
chk "no image: exit 2" test $rc -eq 2

echo "default list"
out=$(bash "$SWEEP" --dry-run "$D/zephyr.bin" "$D/o6" 2>&1)
n=$(sed -n 's/^mp2-deqp-sweep: dry run: \([0-9]*\) cases.*/\1/p' <<<"$out")
want=$(sh "$HERE/mp2-deqp-list-check.sh" | sed -n 's/.*, \([0-9]*\) mustpass cases to run/\1/p')
chk "default: $n cases equal list-check's $want" test -n "$n" -a "$n" = "$want"

echo "$pass ok, $fail failed"
[ "$fail" -eq 0 ]
