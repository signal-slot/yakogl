#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of mp2s_sim_qemu_diff.sh's comparison logic on canned sequences, of
# its simulator-side normalizer on the real simulator, and of its clean skip
# (no QEMU, no board).
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
diff_sh=$here/mp2s_sim_qemu_diff.sh
w=$(mktemp -d "${TMPDIR:-/tmp}/sqdiff-st.XXXXXX")
trap 'rm -rf "$w"' EXIT
ok=0; bad=0
check() { # name want got
	if [ "$2" = "$3" ]; then ok=$((ok + 1)); else bad=$((bad + 1)); echo "FAIL: $1 (got $3, want $2)"; fi
}
canned() { # N file
	local n=$1 f=$2 i
	: > "$f"
	for ((i = 1; i <= n; i++)); do echo "boot $i: left=$((n - i)) loaded go image" >> "$f"; done
	echo "boot $((n + 1)): usedup" >> "$f"
	echo "first-stock: $((n + 1))" >> "$f"
	echo "env: boot_prefixes=/mmc0_ mp2s=none" >> "$f"
}
cmp_rc() { local rc=0; "$diff_sh" --compare "$1" "$2" > "$w/out" || rc=$?; echo "$rc"; }

for n in 1 2 3; do
	canned "$n" "$w/a$n"; cp "$w/a$n" "$w/b$n"
	check "equal N=$n" 0 "$(cmp_rc "$w/a$n" "$w/b$n")"
done

canned 2 "$w/a"
sed 's/left=0/left=1/' "$w/a" > "$w/b"
check "count differs" 1 "$(cmp_rc "$w/a" "$w/b")"
grep -q 'boot 2' "$w/out"; check "names boot 2" 0 "$?"
sed 's/loaded go image/loaded go/' "$w/a" > "$w/b"
check "image missing" 1 "$(cmp_rc "$w/a" "$w/b")"
sed 's/first-stock: 3/first-stock: 2/' "$w/a" > "$w/b"
check "stock boot differs" 1 "$(cmp_rc "$w/a" "$w/b")"
sed 's/mp2s=none/mp2s=mp2s_left=0/' "$w/a" > "$w/b"
check "final env differs" 1 "$(cmp_rc "$w/a" "$w/b")"
sed '/^boot 3/d' "$w/a" > "$w/b"
check "boot missing" 1 "$(cmp_rc "$w/a" "$w/b")"
: > "$w/e"
check "empty sequences" 1 "$(cmp_rc "$w/e" "$w/e")"

# the simulator side: the real simulator's output normalizes to the expected record
cmd=$root/driver/tools/mp2-standalone/mp2-standalone.cmd
for n in 1 2 3; do
	python3 -I "$root/tools/mp2s_script_sim.py" run "$cmd" --left "$n" \
		| "$diff_sh" --normalize-sim > "$w/sim.txt"
	canned "$n" "$w/exp"
	check "simulator N=$n matches the expected record" 0 "$(cmp_rc "$w/sim.txt" "$w/exp")"
done

# no QEMU: a clean skip naming the missing variable
rc=0; out=$(env -u QEMU_SYSTEM_AARCH64 "$diff_sh") || rc=$?
check "skip exit" 0 "$rc"
case $out in skip:*QEMU_SYSTEM_AARCH64*) ok=$((ok + 1)) ;; *) bad=$((bad + 1)); echo "FAIL: skip line: $out" ;; esac
rc=0; out=$(env -u CONNECTED_DEPS_DIR -u UBOOT_V2023_10_SRC QEMU_SYSTEM_AARCH64=/bin/true "$diff_sh") || rc=$?
check "skip exit (no U-Boot)" 0 "$rc"
case $out in skip:*UBOOT_V2023_10_SRC*) ok=$((ok + 1)) ;; *) bad=$((bad + 1)); echo "FAIL: skip line: $out" ;; esac

echo "mp2s_sim_qemu_diff_selftest: $ok ok, $bad failed"
[ "$bad" -eq 0 ]
