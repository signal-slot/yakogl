#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mesa-etna-build_selftest.sh - checks tools/vivante/mesa-etna-build.sh and
# the MESA_ETNA_TREE line of tools/cts/deps-env.sh without Mesa, a network
# or a build (issues #7, #1281).
#
#   tools/vivante/mesa-etna-build_selftest.sh
#
# Prints "N ok, M failed" and exits 0 only if none failed. It covers --check
# on a missing directory, an unbuilt one, a stale marker, a stand-in tree
# that lacks a file and a complete stand-in tree; --dry-run (prints the
# steps, writes nothing); the usage error; and deps-env.sh, which prints
# MESA_ETNA_TREE for a layout whose tree is complete and not otherwise. It
# does NOT build the tree; that is a run by hand (tools/vivante/README.md).

set -u
export LC_ALL=C
unset MESA_ETNA_TREE

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
BUILD=$HERE/mesa-etna-build.sh
DEPS_ENV=$ROOT/tools/cts/deps-env.sh

ok=0
failed=0
pass() { ok=$((ok + 1)); }
fail() {
	failed=$((failed + 1))
	echo "FAIL: $1" >&2
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/mesa_etna_build_selftest.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT

run() {
	"$@" >"$TMP/out" 2>"$TMP/err"
	rc=$?
}

# expect_gap NAME WHY DIR: --check exits 2, one `gap: MESA_ETNA_TREE:` line
# on stderr that contains WHY, nothing on stdout.
expect_gap() {
	local name=$1 why=$2 n
	run "$BUILD" --check "$3"
	n=$(grep -c '^gap: MESA_ETNA_TREE: ' "$TMP/err")
	if [ "$rc" -eq 2 ] && [ "$n" -eq 1 ] && grep -q -- "$why" "$TMP/err" && [ ! -s "$TMP/out" ]; then
		pass
	else
		fail "$name (rc=$rc, gap lines=$n): $(head -c 300 "$TMP/err")"
	fi
}

# stand_in DIR: a tree with every file --check reads and the marker of this
# checkout's inputs.
stand_in() {
	local d=$1 t=$1/mesa-26.2.1 f
	for f in src/etnaviv/hwdb/st/gc_feature_database.h src/etnaviv/hwdb/nxp/gc_feature_database.h \
		src/etnaviv/hw/state_3d.xml.h include/GLES2/gl2.h src/gallium/drivers/etnaviv/etna_shc.c; do
		mkdir -p "$(dirname "$t/$f")" && : >"$t/$f"
	done
	mkdir -p "$t/build-etnaviv/src/gallium/drivers/etnaviv" "$t/src/etnaviv/drm-shim" \
		"$t/build/mesa-etnaviv-host/src/gallium/targets"
	printf '#!/bin/sh\n' >"$t/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc"
	chmod +x "$t/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc"
	echo 'getenv("ETNA_SHIM_CS_DUMP")' >"$t/src/etnaviv/drm-shim/etnaviv_noop.c"
	for f in libetnaviv_noop_drm_shim.so libEGL.so libGLESv2.so; do
		: >"$t/build/mesa-etnaviv-host/src/gallium/targets/$f"
	done
	"$BUILD" --marker >"$d/mesa-etna-build.done"
}

# 1. --check
expect_gap "a missing directory" "no directory" "$TMP/nope"
mkdir -p "$TMP/empty"
expect_gap "a directory without a marker" "no marker" "$TMP/empty"

stand_in "$TMP/good"
run "$BUILD" --check "$TMP/good"
if [ "$rc" -eq 0 ] && [ "$(cat "$TMP/out")" = "MESA_ETNA_TREE=$TMP/good/mesa-26.2.1" ]; then
	pass
else
	fail "a complete stand-in tree (rc=$rc): $(head -c 300 "$TMP/out" "$TMP/err")"
fi

stand_in "$TMP/stale"
echo "input etna_shc.c 0000" >>"$TMP/stale/mesa-etna-build.done"
expect_gap "a marker of other inputs" "is stale" "$TMP/stale"

stand_in "$TMP/nopatch"
: >"$TMP/nopatch/mesa-26.2.1/src/etnaviv/drm-shim/etnaviv_noop.c"
expect_gap "a tree without mesa-cs-golden.patch" "not patched with mesa-cs-golden.patch" "$TMP/nopatch"

stand_in "$TMP/noshim"
rm "$TMP/noshim/mesa-26.2.1/build/mesa-etnaviv-host/src/gallium/targets/libetnaviv_noop_drm_shim.so"
expect_gap "a tree without the drm-shim" "no libetnaviv_noop_drm_shim.so" "$TMP/noshim"

stand_in "$TMP/noshc"
rm "$TMP/noshc/mesa-26.2.1/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc"
expect_gap "a tree without etna_shc" "no etna_shc" "$TMP/noshc"

# The marker names every input file by its sha256.
run "$BUILD" --marker
if [ "$rc" -eq 0 ] && grep -q '^mesa 26.2.1 [0-9a-f]\{64\}$' "$TMP/out" &&
	[ "$(grep -c '^input .* [0-9a-f]\{64\}$' "$TMP/out")" -eq 3 ] &&
	[ "$(grep -c '^sdist ' "$TMP/out")" -eq 5 ]; then
	pass
else
	fail "--marker (rc=$rc)"
fi

# 2. --dry-run: the steps of a build, nothing written.
run "$BUILD" --dry-run "$TMP/dry"
if [ "$rc" -eq 0 ] && [ ! -e "$TMP/dry" ] &&
	grep -q '^== fetch https://archive.mesa3d.org/mesa-26.2.1.tar.xz' "$TMP/out" &&
	grep -q 'mesa-etna_shc.patch' "$TMP/out" && grep -q 'mesa-cs-golden.patch' "$TMP/out" &&
	grep -q 'ninja -C .*build-etnaviv' "$TMP/out" && grep -q 'ninja -C .*build/mesa-etnaviv-host' "$TMP/out"; then
	pass
else
	fail "--dry-run (rc=$rc, wrote: $([ -e "$TMP/dry" ] && echo yes || echo no))"
fi
run "$BUILD" --dry-run "$TMP/good"
if [ "$rc" -eq 0 ] && grep -q 'nothing to do' "$TMP/out"; then pass; else fail "--dry-run on a complete tree (rc=$rc)"; fi

# 3. The usage error.
run "$BUILD" --bogus
if [ "$rc" -eq 64 ] && grep -q -- '--check' "$TMP/err"; then pass; else fail "an unknown option (rc=$rc)"; fi

# 4. deps-env.sh: MESA_ETNA_TREE only for a complete tree of the layout.
mkdir -p "$TMP/layout"
run env CONNECTED_DEPS_DIR="$TMP/layout" CCFARM_DEPS_DIR="$TMP/layout" "$DEPS_ENV" --present
if [ "$rc" -eq 0 ] && ! grep -q '^MESA_ETNA_TREE=' "$TMP/out"; then pass; else fail "deps-env.sh --present, no tree (rc=$rc)"; fi
stand_in "$TMP/layout/mesa-etna-26.2.1"
for form in --present ""; do
	# shellcheck disable=SC2086  # the empty form is no argument
	run env CONNECTED_DEPS_DIR="$TMP/layout" CCFARM_DEPS_DIR="$TMP/layout" "$DEPS_ENV" $form
	if [ "$rc" -eq 0 ] && [ "$(grep -c '^MESA_ETNA_TREE=' "$TMP/out")" -eq 1 ] &&
		grep -qx "MESA_ETNA_TREE=$TMP/layout/mesa-etna-26.2.1/mesa-26.2.1" "$TMP/out"; then
		pass
	else
		fail "deps-env.sh ${form:-(no option)}, a complete tree (rc=$rc)"
	fi
done
run env CONNECTED_DEPS_DIR="$TMP/layout" CCFARM_DEPS_DIR="$TMP/layout" "$DEPS_ENV" --layout
if [ "$rc" -eq 0 ] && [ "$(cat "$TMP/out")" = "$TMP/layout" ]; then pass; else fail "deps-env.sh --layout prints the layout only (rc=$rc)"; fi

echo "mesa-etna-build_selftest: $ok ok, $failed failed"
[ "$failed" -eq 0 ]
