#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-board.sh's chain stop: the wrapper runs a table entry by entry through a stub
# board.sh (IMX8MP_GLES3_BOARD_SH). Cases: all ok runs every entry; rc=5 not_back at entry 2 stops there
# with non-zero; a refused jump (rc=3 or a verdict=refused line) stops; a SIGTERM stops the running child.
# Host only; writes under ${TMPDIR:-$PWD}.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-$PWD}/gles3-board-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

root=$tmp/root
mkdir -p "$root/tools/cts"
cp "$here/imx8mp-gles3-board.sh" "$root/tools/cts/"
printf '# table\ndEQP-A.*\ndEQP-B.*\ndEQP-C.*\ndEQP-D.*\n' > "$root/tools/cts/imx8mp-gles3-tree.txt"
cat > "$tmp/stub.sh" <<'STUB'
#!/bin/bash
# stub board.sh: MODE_<row> = "rc verdict" decides the run of a row (default "0 done")
row=${TREE_ROWS:-all}
echo "$row" >> "$STUB_LOG"
m=MODE_$row; set -- ${!m:-0 done}
if [ "$2" = hang ]; then echo "$row hanging"; sleep 30; exit 0; fi
echo "board.sh x2 e0$row rc=$1 verdict=$2 boot=x"
exit "$1"
STUB
chmod +x "$tmp/stub.sh" "$root/tools/cts/imx8mp-gles3-board.sh"
w=$root/tools/cts/imx8mp-gles3-board.sh
run() { : > "$tmp/ran"; env IMX8MP_GLES3_BOARD_SH="$tmp/stub.sh" STUB_LOG="$tmp/ran" TREE_IMAGE=/x.bin "$@" "$w" --board x2 tree "$tmp/out" > "$tmp/o" 2> "$tmp/e"; }

run; rc=$?
if [ $rc -eq 0 ] && [ "$(tr '\n' ' ' < "$tmp/ran")" = "0 1 2 3 " ] && ! grep -q STOP "$tmp/e"; then ok "all ok: four entries, exit 0"
else bad "all ok: rc=$rc ran=$(tr '\n' ' ' < "$tmp/ran")"; fi

run MODE_1="5 not_back"; rc=$?
if [ $rc -eq 5 ] && [ "$(tr '\n' ' ' < "$tmp/ran")" = "0 1 " ] && grep -q 'STOP e01 rc=5 verdict=not_back' "$tmp/e"; then ok "rc=5 not_back at entry 1: stops there, exit 5"
else bad "not_back: rc=$rc ran=$(tr '\n' ' ' < "$tmp/ran") $(cat "$tmp/e")"; fi

run MODE_2="3 refused"; rc=$?
if [ $rc -eq 3 ] && [ "$(tr '\n' ' ' < "$tmp/ran")" = "0 1 2 " ] && grep -q 'STOP e02 rc=3 verdict=refused' "$tmp/e"; then ok "refused jump: stops, exit 3"
else bad "refused: rc=$rc ran=$(tr '\n' ' ' < "$tmp/ran") $(cat "$tmp/e")"; fi

run MODE_1="0 refused"; rc=$?
if [ $rc -ne 0 ] && [ "$(tr '\n' ' ' < "$tmp/ran")" = "0 1 " ]; then ok "a refused verdict with rc 0 still stops, non-zero"
else bad "refused rc0: rc=$rc"; fi

run MODE_1="4 failed"; rc=$?
if [ $rc -ne 0 ] && [ "$(tr '\n' ' ' < "$tmp/ran")" = "0 1 " ]; then ok "any non-zero board.sh exit stops"
else bad "rc4: rc=$rc"; fi

run TREE_ROWS=2-3; rc=$?
if [ $rc -eq 0 ] && [ "$(tr '\n' ' ' < "$tmp/ran")" = "2 3 " ]; then ok "TREE_ROWS=2-3 runs entries 2 and 3 only"
else bad "TREE_ROWS: rc=$rc ran=$(tr '\n' ' ' < "$tmp/ran")"; fi

# SIGTERM: the wrapper ends, non-zero, the hanging child is gone
: > "$tmp/ran"
IMX8MP_GLES3_BOARD_SH="$tmp/stub.sh" STUB_LOG="$tmp/ran" TREE_IMAGE=/x.bin MODE_1="0 hang" "$w" --board x2 tree "$tmp/out" > "$tmp/o" 2> "$tmp/e" &
wp=$!
for _ in $(seq 50); do grep -q hanging "$tmp/o" 2>/dev/null && break; sleep 0.1; done
kill -TERM "$wp"; wait "$wp"; rc=$?
if [ $rc -eq 143 ] && grep -q STOPPED "$tmp/e"; then ok "SIGTERM: stops the chain, exit 143"
else bad "SIGTERM: rc=$rc $(cat "$tmp/e")"; fi
sleep 0.3
if pgrep -f "$tmp/stub.sh" > /dev/null; then bad "SIGTERM: the stub child still runs"; else ok "SIGTERM: the child is gone"; fi

[ $nfail -eq 0 ] && echo "imx8mp-gles3-board-selftest: all ok" || echo "imx8mp-gles3-board-selftest: $nfail failed" >&2
exit $((nfail > 0))
