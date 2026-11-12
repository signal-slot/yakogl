#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gate_st_selftest.sh - checks tools/vivante/gate_st.sh without a Mesa tree.
#
#   tools/vivante/gate_st_selftest.sh
#
# Prints "N ok, M failed" and exits 0 only if none failed. It covers the gap
# path, the usage error, synthetic trees that are not real Mesa trees, and the
# pinned mode list. It does NOT run `golden` or the hwdb pass path against a
# real tree; that needs MESA_ETNA_TREE (see gate_st.sh).

set -u
# The checks read the scripts' messages, so the locale must not matter.
export LC_ALL=C
unset MESA_ETNA_TREE MESA_ETNA_BUILD GATE_ST_WANT_FILE

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GATE=$HERE/gate_st.sh

ok=0
failed=0
pass() { ok=$((ok + 1)); }
fail() {
	failed=$((failed + 1))
	echo "FAIL: $1" >&2
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/gate_st_selftest.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT

# run_gate NAME ARGS...: the exit code in $rc, stdout in $TMP/out, stderr in
# $TMP/err. The caller sets any environment with `env` in front of ARGS.
run_gate() {
	"$@" >"$TMP/out" 2>"$TMP/err"
	rc=$?
}

# expect_gap NAME ARGS...: exit 2, exactly one stderr line that starts with
# `gap:`, naming MESA_ETNA_TREE and saying the check was NOT run, and nothing
# on stdout. Other stderr lines are not counted (the shell may add its own).
expect_gap() {
	local name=$1 n
	shift
	run_gate "$@"
	n=$(grep -c '^gap:' "$TMP/err")
	if [ "$rc" -eq 2 ] && [ "$n" -eq 1 ] &&
		grep -q '^gap: .*MESA_ETNA_TREE' "$TMP/err" &&
		grep -q 'NOT run' "$TMP/err" && [ ! -s "$TMP/out" ]; then
		pass
	else
		fail "$name (rc=$rc, gap lines=$n)"
	fi
}

# The gap path: MESA_ETNA_TREE unset, for both subcommands.
expect_gap "hwdb without a tree" "$GATE" hwdb
expect_gap "golden without a tree" "$GATE" golden

# A directory that does not exist.
expect_gap "hwdb, missing directory" env MESA_ETNA_TREE="$TMP/nope" "$GATE" hwdb
expect_gap "golden, missing directory" env MESA_ETNA_TREE="$TMP/nope" "$GATE" golden

# A directory that is not a Mesa tree.
mkdir "$TMP/empty"
expect_gap "hwdb, not a Mesa tree" env MESA_ETNA_TREE="$TMP/empty" "$GATE" hwdb
expect_gap "golden, not a Mesa tree" env MESA_ETNA_TREE="$TMP/empty" "$GATE" golden

# A synthetic tree whose two hwdb headers exist but contain no rows. hwdb must
# not pass: hwdb_compare.py finds no matching row and exits 1 with a message
# (never a traceback). golden has no build directory there, so it is a gap.
HW=$TMP/hollow
mkdir -p "$HW/src/etnaviv/hwdb/st" "$HW/src/etnaviv/hwdb/nxp"
: >"$HW/src/etnaviv/hwdb/st/gc_feature_database.h"
: >"$HW/src/etnaviv/hwdb/nxp/gc_feature_database.h"
run_gate env MESA_ETNA_TREE="$HW" "$GATE" hwdb
if [ "$rc" -eq 1 ] && grep -q '0 rows match' "$TMP/err" "$TMP/out" &&
	! grep -q Traceback "$TMP/err" "$TMP/out"; then
	pass
else
	fail "hwdb on empty headers: want exit 1 and '0 rows match' with no Traceback (rc=$rc)"
fi
expect_gap "golden, empty headers and no build" env MESA_ETNA_TREE="$HW" "$GATE" golden

# A build directory without the shim (and without the rest).
mkdir -p "$TMP/build"
expect_gap "golden, build without the shim" env MESA_ETNA_TREE="$HW" MESA_ETNA_BUILD="$TMP/build" "$GATE" golden

# The usage error.
for args in "" "bogus" "hwdb golden" "HWDB" "hwdb;true"; do
	# shellcheck disable=SC2086 # word splitting of the literal list is the point
	run_gate "$GATE" $args
	if [ "$rc" -eq 64 ]; then pass; else fail "usage error for '$args' (rc=$rc)"; fi
done
run_gate env MESA_ETNA_TREE="$TMP/empty" "$GATE" bogus
if [ "$rc" -eq 64 ]; then pass; else fail "usage error is checked before the tree (rc=$rc)"; fi

# golden pins exactly three modes: 1, 8 and 15.
modes=$(grep -c '^mode [0-9][0-9]* renderer: ' "$GATE")
list=$(grep '^mode [0-9]' "$GATE" | sed "s/^mode \([0-9]*\) .*/\1/" | tr '\n' ' ')
if [ "$modes" -eq 3 ] && [ "$list" = "1 8 15 " ]; then
	pass
else
	fail "golden pins modes '$list' ($modes lines), want 1 8 15"
fi

# cs_decode.py on a GC8000 stream: a pipe select at 0x03800 (a TS field mask
# shared the number), PS_END_PC at 0x01000 (a fill-mode flag did), a BLT state,
# and the commands that take operand words (DRAW_INSTANCED is how Mesa draws on
# a Halti2+ part: its count and start were printed as commands of their own).
# The register names come from the headers committed under driver/src/vivante/hw.
ROOT=$(cd "$HERE/../.." && pwd)
HW=$ROOT/driver/src/vivante/hw
python3 -I - "$TMP/gc8000.raw" <<'PY'
import struct, sys
def ls(a, v):
    w = [(1 << 27) | (len(v) << 16) | (a // 4)] + v
    return w + [0] * (len(w) % 2)
w = (ls(0x3800, [0]) + ls(0x1000, [7]) + ls(0x140b8, [1]) + [0x60040001, 3, 0, 0]
     + [0x78000002, 0xffff1000] + [0x40000002, 0x2000]
     + [0x50000004, 0x3000, 4, 0x4000] + [0x10000000, 0])
open(sys.argv[1], "wb").write(struct.pack("<%uI" % len(w), *w))
PY
run_gate python3 -I "$HERE/cs_decode.py" "$TMP/gc8000.raw" --raw --regs "$HW/state_3d.xml.h" --regs2 "$HW/state.xml.h"
if [ "$rc" -eq 0 ] && diff "$TMP/out" "$HERE/testdata/gc8000-stream.golden" >"$TMP/diff"; then
	pass
else
	fail "cs_decode.py on a GC8000 stream differs from testdata/gc8000-stream.golden (rc=$rc)"
	head -20 "$TMP/diff" >&2
fi

# A missing register header is said on stderr, not silently printed as addresses.
run_gate python3 -I "$HERE/cs_decode.py" "$TMP/gc8000.raw" --raw --regs "$TMP/nope.h" --regs2 "$TMP/nope2.h"
if [ "$rc" -eq 0 ] && grep -q '^warning: --regs ' "$TMP/err"; then
	pass
else
	fail "cs_decode.py with a missing header: want a warning on stderr (rc=$rc)"
fi

# No dump at all is an exit status, not a trace.
run_gate python3 -I "$HERE/cs_decode.py" "$TMP/no-such-dump"
if [ "$rc" -ne 0 ] && ! grep -q Traceback "$TMP/err"; then pass; else fail "cs_decode.py without a dump (rc=$rc)"; fi

# golden against a stand-in tree: the build directory is found as build-host,
# and cs_golden runs with the etnaviv loader forced (the stub fails without it,
# as a host with a GPU of its own does).
GT=$TMP/goldtree
mkdir -p "$GT/src/etnaviv/hw" "$GT/src/etnaviv/drm-shim" "$GT/include/GLES2" \
	"$GT/build-host/lib" "$GT/bin"
: >"$GT/src/etnaviv/hw/state_3d.xml.h"
: >"$GT/include/GLES2/gl2.h"
echo ETNA_SHIM_CS_DUMP >"$GT/src/etnaviv/drm-shim/etnaviv_noop.c"
: >"$GT/build-host/lib/libetnaviv_noop_drm_shim.so"
: >"$GT/build-host/lib/libEGL.so"
: >"$GT/build-host/lib/libGLESv2.so"
cat >"$GT/bin/cc" <<'CC'
#!/bin/sh
# stands in for the compiler: the -o file is a cs_golden that needs the loader override
while [ $# -gt 0 ]; do
	if [ "$1" = -o ]; then out=$2; fi
	shift
done
cat >"$out" <<'STUB'
#!/bin/sh
if [ "${MESA_LOADER_DRIVER_OVERRIDE:-}" != etnaviv ]; then
	echo "libEGL warning: egl: failed to create dri2 screen" >&2
	exit 1
fi
echo "renderer: Vivante GC8000 rev 6205" >&2
: >"$ETNA_SHIM_CS_DUMP"
STUB
chmod +x "$out"
CC
chmod +x "$GT/bin/cc"
for m in 1 8 15; do
	echo "mode $m renderer: Vivante GC8000 rev 6205 pe_mem_config=same ts_mem_config=same other=0"
done >"$TMP/want"
run_gate env MESA_ETNA_TREE="$GT" CC="$GT/bin/cc" GATE_ST_WANT_FILE="$TMP/want" "$GATE" golden
if [ "$rc" -eq 0 ] && grep -q 'signature matches' "$TMP/out"; then
	pass
else
	fail "golden on a stand-in tree with build-host (rc=$rc)"
	cat "$TMP/out" "$TMP/err" >&2
fi

# Both scripts parse.
for s in "$GATE" "$HERE/gate_st_selftest.sh"; do
	if bash -n "$s"; then pass; else fail "bash -n $s"; fi
done

echo "$ok ok, $failed failed"
[ "$failed" -eq 0 ]
