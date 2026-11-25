#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-uboot-rows-selftest.sh - t6-uboot-rows.sh with a fake prompt-go.
#
#   bash tools/cts/t6-uboot-rows-selftest.sh
if [ -z "$BASH_VERSION" ]; then exec bash "$0" "$@"; fi
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/t6rows.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT
fails=0
# check <name> <command...>: ok when the command succeeds
check() {
	local n=$1
	shift
	if "$@"; then echo "ok: $n"; else echo "FAIL: $n"; fails=$((fails + 1)); fi
}
eq() { [ "$1" = "$2" ]; }
lines() { wc -l < "$1" | tr -d ' '; }

cat > "$T/tree.txt" <<'X'
# image T6S1R01: 1
# image T6S1R02: 2
# image T6S1R03: 3
# image T6S1R04: 4
# image T6S1R05: 5
X
for l in T6S1R01 T6S1R02 T6S1R03 T6S1R04; do
	mkdir -p "$T/img/$l/zephyr"
	echo "$l" > "$T/img/$l/zephyr/zephyr.bin"
done
mkdir -p "$T/img/T6S1R05" # label known, no zephyr.bin

# Fake prompt-go: its exit code is the first line of $FAKE_CODES (consumed).
cat > "$T/fake.sh" <<'X'
#!/bin/sh
echo "$@" >> "$FAKE_CALLS"
code=$(head -n 1 "$FAKE_CODES"); sed -i 1d "$FAKE_CODES"
echo "prompt-go.sh: record /rec/$(cat "$1")"
exit "$code"
X
chmod +x "$T/fake.sh"
export PROMPT_GO=$T/fake.sh TREE=$T/tree.txt FAKE_CALLS=$T/calls FAKE_CODES=$T/codes
R=$HERE/t6-uboot-rows.sh
run() { bash "$R" "$@" > "$T/stdout" 2> "$T/stderr"; }
row() { awk -F'\t' -v l="$1" -v c="$2" -v r="$3" '$1 == l && $2 == c && $4 == r { f = 1 } END { exit !f }' "$4"; }

: > "$FAKE_CALLS"
run "$T/img" "$T/o1" T6S1R01 NOPE
check "unknown label refused with 2" eq $? 2
check "unknown label: nothing ran" test ! -s "$FAKE_CALLS"

run "$T/img" "$T/o1" T6S1R01 T6S1R05
check "missing image refused with 2" eq $? 2
check "missing image: nothing ran" test ! -s "$FAKE_CALLS"

printf '0\n11\n0\n' > "$FAKE_CODES"
run "$T/img" "$T/o2" T6S1R01 T6S1R02 T6S1R03 --go
check "0/11/0 run exits 0" eq $? 0
check "0/11/0 gives three rows" eq "$(lines "$T/o2/rows.tsv")" 3
check "row 2 is 11 with its record" row T6S1R02 11 /rec/T6S1R02 "$T/o2/rows.tsv"
check "--go passed on" grep -q -- '--go' "$FAKE_CALLS"
check "deqp probe passed" grep -q ' deqp' "$FAKE_CALLS"

: > "$FAKE_CALLS"
printf '0\n10\n0\n' > "$FAKE_CODES"
run "$T/img" "$T/o3" T6S1R01 T6S1R02 T6S1R03 T6S1R04
check "stop at 10 exits 3" eq $? 3
check "stop at 10: two rows" eq "$(lines "$T/o3/rows.tsv")" 2
check "stop at 10: two calls, later labels unrun" eq "$(lines "$FAKE_CALLS")" 2
check "stop says a person may be needed" grep -q 'may need a person' "$T/stderr"
check "no --go without --go" test -z "$(grep -- '--go' "$FAKE_CALLS")"

# resume: R01 done (0), R02 stopped at 10 (not done): R02..R04 run.
: > "$FAKE_CALLS"
printf '0\n11\n0\n' > "$FAKE_CODES"
run "$T/img" "$T/o3" T6S1R01 T6S1R02 T6S1R03 T6S1R04
check "resume exits 0" eq $? 0
check "resume ran three rows" eq "$(lines "$FAKE_CALLS")" 3
check "resume skipped the done row" test -z "$(grep T6S1R01 "$FAKE_CALLS")"
check "R01 once in rows.tsv" eq "$(cut -f1 "$T/o3/rows.tsv" | grep -c T6S1R01)" 1

if [ "$fails" = 0 ]; then echo "t6-uboot-rows-selftest: all ok"; exit 0; fi
echo "t6-uboot-rows-selftest: $fails failed"
exit 1
