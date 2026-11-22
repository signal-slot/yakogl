#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# rehearse.sh: rehearsal of the c6 verdict. Runs case a_default_2 (run.sh),
# joins its boot 1..3 console logs into one record and feeds it to
# tools/mp2s_record_check.py --count 2, so drift between the script's console
# lines and the verdict's regexes shows before the board run. Prints the
# verdict's output including its VERDICT line; exit is record_check's.
# Inputs as run.sh. Missing input: prints "skip: ..." and exits 0.
# QEMU has no BL2, so the record has SYNTHETIC separators (README.md).
# MP2S_REHEARSE_LOG=FILE keeps the joined record there.
set -euo pipefail
pdir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$pdir/../../../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/rehearse.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

rc=0
MP2S_PRODUCT_ONLY=a_default_2 MP2S_QEMU_KEEP=1 bash "$pdir/run.sh" > "$tmp/run.out" 2>&1 || rc=$?
if grep -q '^SKIP:' "$tmp/run.out"; then
	echo "skip: $(grep -m1 '^SKIP:' "$tmp/run.out" | sed 's/^SKIP: //')"
	exit 0
fi
tail -n 3 "$tmp/run.out"
[ "$rc" -eq 0 ] || { echo "rehearse.sh: run.sh failed ($rc); no record"; exit 1; }
logs=$(sed -n 's/^run.sh: logs in //p' "$tmp/run.out" | tail -n 1)
[ -d "$logs" ] || { echo "rehearse.sh: no logs directory"; exit 1; }

# SYNTHETIC: one BL2-banner line before each of boots 1..3 (boot 4 is the
# stock-only check, not part of the c6 record). Every other line is QEMU's.
rec=${MP2S_REHEARSE_LOG:-$tmp/record.log}
: > "$rec"
for n in 1 2 3; do
	printf 'NOTICE:  BL2: v2.10-synthetic (rehearsal boot %s)\r\n' "$n" >> "$rec"
	cat "$logs/a_default_2-boot$n.log" >> "$rec"
done
rm -rf "$(dirname "$logs")"
img='mp2q: image ran at its link address 0xa0000000'
stock='mp2q: stock path /mmc0_ reached'
python3 -I "$root/tools/mp2s_record_check.py" "$rec" --count 2 --image-line "$img" --linux-line "$stock"
