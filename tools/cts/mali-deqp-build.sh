#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build dEQP-GLES2 for the NanoPC-T6's Armbian (Debian trixie, aarch64; Mali-G610
# over panthor), into the build directory tools/cts/mali-deqp-run.sh reads.
#
#   tools/cts/mali-deqp-build.sh --out DIR [--gles-lib DIR] [--dry-run] [--allow-dirty]
#
# deqp-gles2 is VK-GL-CTS at CTS_COMMIT with cts-yakogl-target.patch: a
# dynamically linked aarch64 Linux program that links no GL or EGL library and
# dlopen()s libGLESv2.so.2 and libEGL.so.1 through LD_LIBRARY_PATH at run time,
# headless, on pbuffer surfaces (--deqp-surface-type=pbuffer), as
# host-deqp-run.sh runs it on a host.
#
# Output, DIR absent or empty beforehand (written as DIR.part, renamed at the end):
#   DIR/deqp-gles2      the program
#   DIR/data/gles2/     its data (dEQP's archive directory is DIR/data)
#   DIR/mustpass        tools/cts/gles2-mustpass.txt
#   DIR/lib/            libGLESv2.so.2 and libEGL.so.1, copied from --gles-lib
#                       (tools/mali/panthor-gles/build.sh makes them; not here)
#   DIR/build.txt       commit=<HEAD, 40 hex>, dirty=, cts=<CTS_COMMIT>,
#                       toolchain=, glibc_max=, glibcxx_max=, cxxabi_max=,
#                       build_seconds=
#
# The board's ABI floors (Armbian trixie, GCC 14.2 userspace): GLIBC 2.41,
# GLIBCXX 3.4.33, CXXABI 1.3.15. The compiler is Debian trixie's own
# g++-14-aarch64-linux-gnu with its cross glibc 2.41 and libstdc++, the pinned
# packages of MALI_DEQP_TOOLCHAIN in tools/cts/deps.toml (unpacked without root
# or docker). After the build, every version deqp-gles2 needs in its dynamic
# symbols (objdump -T) is checked against the floors, and the build fails when
# one is newer; it fails too when the program is not AArch64, does not use
# /lib/ld-linux-aarch64.so.1, or links libGLESv2/libEGL directly.
#
# The working tree must be clean (no change to a tracked file), so build.txt's
# commit is the one the code came from. --allow-dirty builds anyway, for local
# tests only: build.txt then says dirty=yes and the result is not a record.
# --dry-run checks the arguments and the tree and prints every step, running
# none (no toolchain, CTS or build directory is needed for it).
#
# Environment:
#   CTS        VK-GL-CTS clone that has CTS_COMMIT, with its external sources
#              fetched (external/fetch_sources.py); never written to
#              (default <deps>/VK-GL-CTS, the clone DEQP_WORK of deps.toml makes)
#   TOOLCHAIN  MALI_DEQP_TOOLCHAIN (default <deps>/debian-trixie-arm64-cross;
#              made with `tools/cts/deps-env.sh --build --board nanopc_t6
#              MALI_DEQP_TOOLCHAIN` when the default is absent)
#   BUILD      build tree (default <repo>/build/cts/mali-aarch64)
#   JOBS       parallel compile jobs (default nproc)
# <deps> is the dependency layout, tools/cts/deps-env.sh --layout.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
me=mali-deqp-build
CTS_COMMIT=93bca01861b0e3ef3c387027a9791e6d065f900c
GLIBC_FLOOR=2.41
GLIBCXX_FLOOR=3.4.33
CXXABI_FLOOR=1.3.15
PATCH=$HERE/cts-yakogl-target.patch
MUSTPASS=$HERE/gles2-mustpass.txt

usage() {
	sed -n '8p' "$0" | sed 's/^# \{0,1\}//' >&2
	exit 2
}

out= gles_lib= dry=0 allow_dirty=0
while [ $# -gt 0 ]; do
	case $1 in
	--out) out=${2:?--out needs a directory}; shift 2 ;;
	--gles-lib) gles_lib=${2:?--gles-lib needs a directory}; shift 2 ;;
	--dry-run) dry=1; shift ;;
	--allow-dirty) allow_dirty=1; shift ;;
	-h | --help) sed -n '5,49p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	*) echo "$me: unknown argument '$1'" >&2; usage ;;
	esac
done
[ -n "$out" ] || usage

# --- 0. the arguments and the tree --------------------------------------------
if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
	echo "$me: $out exists and is not empty (a build directory is never overwritten)" >&2
	exit 1
fi
if [ -n "$gles_lib" ]; then
	for l in libGLESv2.so.2 libEGL.so.1; do
		[ -f "$gles_lib/$l" ] || { echo "$me: --gles-lib $gles_lib has no $l" >&2; exit 1; }
	done
	gles_lib=$(cd "$gles_lib" && pwd)
fi
commit=$(git -C "$REPO" rev-parse --verify HEAD)
dirty=no
if [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no)" ]; then
	if [ "$allow_dirty" = 1 ]; then
		dirty=yes
		echo "$me: the working tree has changes (--allow-dirty): a local test build, not a record" >&2
	else
		echo "$me: the working tree has changes to tracked files; build.txt's commit=$commit would not be" >&2
		echo "$me: the code's. Commit or stash them (--allow-dirty builds anyway, for local tests only)." >&2
		exit 1
	fi
fi

deps=$("$HERE/deps-env.sh" --layout)
CTS=${CTS:-$deps/VK-GL-CTS}
toolchain_default=$deps/debian-trixie-arm64-cross
TOOLCHAIN=${TOOLCHAIN:-$toolchain_default}
BUILD=${BUILD:-$REPO/build/cts/mali-aarch64}
JOBS=${JOBS:-$(nproc)}
mkdir_out=$(dirname "$out")
out_abs=$( [ -d "$mkdir_out" ] && cd "$mkdir_out" && pwd || echo "$mkdir_out")/$(basename "$out")
part=$out_abs.part

# A step: printed always, run unless --dry-run.
run() {
	printf '+'
	printf ' %q' "$@"
	printf '\n'
	[ "$dry" = 1 ] || "$@"
}
step() { printf '== %s\n' "$*"; }

[ "$dry" = 0 ] || echo "$me: --dry-run: the steps below are printed, none is run"
echo "$me: commit=$commit dirty=$dirty cts=$CTS_COMMIT"
echo "$me: CTS=$CTS TOOLCHAIN=$TOOLCHAIN BUILD=$BUILD out=$out_abs"

# --- 1. the toolchain -----------------------------------------------------------
gxx=$TOOLCHAIN/bin/aarch64-linux-gnu-g++
step "1. the Debian trixie arm64 cross toolchain (MALI_DEQP_TOOLCHAIN, tools/cts/deps.toml)"
if [ ! -x "$gxx" ]; then
	if [ "$TOOLCHAIN" = "$toolchain_default" ]; then
		run "$HERE/deps-env.sh" --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN
	elif [ "$dry" = 0 ]; then
		echo "$me: TOOLCHAIN=$TOOLCHAIN has no bin/aarch64-linux-gnu-g++" >&2
		exit 1
	fi
else
	echo "present: $gxx"
fi
if [ "$dry" = 0 ]; then
	[ -x "$gxx" ] || { echo "$me: no $gxx after the toolchain step" >&2; exit 1; }
	toolchain=$("$gxx" --version | sed -n 1p)
	echo "$me: $toolchain"
fi

# --- 2. a private CTS copy with the yakogl target --------------------------------
# The CTS clone is never written to: a --shared clone of CTS_COMMIT goes to
# $BUILD/cts, the clone's fetched external sources are linked in, and the
# patch is applied there (as cts-build-zephyr.sh does). The copy is made again
# when the patch changes.
step "2. VK-GL-CTS $CTS_COMMIT with cts-yakogl-target.patch in $BUILD/cts"
if [ "$dry" = 0 ]; then
	git -C "$CTS" cat-file -e "$CTS_COMMIT^{commit}" 2>/dev/null || {
		echo "$me: $CTS has no VK-GL-CTS commit $CTS_COMMIT. Make the clone as DEQP_WORK's build does:" >&2
		echo "  git clone --no-checkout https://github.com/KhronosGroup/VK-GL-CTS $CTS" >&2
		echo "  git -C $CTS checkout --detach $CTS_COMMIT" >&2
		echo "  (cd $CTS && python3 external/fetch_sources.py)" >&2
		exit 1
	}
	[ -d "$CTS/external/zlib/src" ] && [ -d "$CTS/external/libpng/src" ] || {
		echo "$me: $CTS has no external sources: (cd $CTS && python3 external/fetch_sources.py)" >&2
		exit 1
	}
fi
patch_sum=$(sha256sum "$PATCH" | cut -c1-64)
if [ "$(cat "$BUILD/cts/.yakogl-patch" 2>/dev/null)" != "$patch_sum" ]; then
	run rm -rf "$BUILD/cts" "$BUILD/build"
	run mkdir -p "$BUILD"
	run git clone -q --shared --no-checkout "$CTS" "$BUILD/cts"
	run git -C "$BUILD/cts" checkout -q --detach "$CTS_COMMIT"
	if [ "$dry" = 1 ]; then
		echo "+ ln -s $CTS/external/<each>/src $BUILD/cts/external/<each>/src"
	else
		for d in "$CTS"/external/*/src; do
			n=$(basename "$(dirname "$d")")
			mkdir -p "$BUILD/cts/external/$n"
			ln -s "$d" "$BUILD/cts/external/$n/src"
		done
	fi
	run git -C "$BUILD/cts" apply "$PATCH"
	[ "$dry" = 1 ] || echo "$patch_sum" > "$BUILD/cts/.yakogl-patch"
fi

# --- 3. configure and build -----------------------------------------------------
# The toolchain file: trixie's drivers (they pass --sysroot themselves), and
# find_* only inside the toolchain, so zlib and libpng come from the CTS's own
# external/ copies, never from the host.
step "3. configure and build deqp-gles2 in $BUILD/build"
tcfile=$BUILD/toolchain-trixie-aarch64.cmake
if [ "$dry" = 1 ]; then
	echo "+ write $tcfile (CMAKE_SYSTEM_NAME Linux, aarch64, $TOOLCHAIN/bin/aarch64-linux-gnu-{gcc,g++,ar,ranlib,strip}, sysroot $TOOLCHAIN)"
else
	mkdir -p "$BUILD"
	# written only when it changes: a new mtime makes CMake configure again
	cat > "$tcfile.new" <<EOF
# Generated by tools/cts/mali-deqp-build.sh: Debian trixie's arm64 cross toolchain.
set(CMAKE_SYSTEM_NAME      Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER   "$TOOLCHAIN/bin/aarch64-linux-gnu-gcc")
set(CMAKE_CXX_COMPILER "$TOOLCHAIN/bin/aarch64-linux-gnu-g++")
set(CMAKE_AR     "$TOOLCHAIN/bin/aarch64-linux-gnu-ar"     CACHE FILEPATH "ar")
set(CMAKE_RANLIB "$TOOLCHAIN/bin/aarch64-linux-gnu-ranlib" CACHE FILEPATH "ranlib")
set(CMAKE_STRIP  "$TOOLCHAIN/bin/aarch64-linux-gnu-strip"  CACHE FILEPATH "strip")
set(CMAKE_SYSROOT "$TOOLCHAIN")
set(CMAKE_FIND_ROOT_PATH "$TOOLCHAIN/usr/aarch64-linux-gnu" "$TOOLCHAIN")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
EOF
	if cmp -s "$tcfile.new" "$tcfile"; then rm "$tcfile.new"; else mv "$tcfile.new" "$tcfile"; fi
fi
run cmake -S "$BUILD/cts" -B "$BUILD/build" -G Ninja \
	-DCMAKE_TOOLCHAIN_FILE="$tcfile" \
	-DCMAKE_BUILD_TYPE=Release \
	-DDEQP_TARGET=yakogl \
	-DSELECTED_BUILD_TARGETS=deqp-gles2
t0=$(date +%s)
run cmake --build "$BUILD/build" --target deqp-gles2 -- -j"$JOBS"
build_seconds=$(( $(date +%s) - t0 ))
bin=$BUILD/build/modules/gles2/deqp-gles2
data=$BUILD/build/modules/gles2/gles2

# --- 4. the floors --------------------------------------------------------------
step "4. check $bin: AArch64, /lib/ld-linux-aarch64.so.1, no GL/EGL link, objdump -T within GLIBC $GLIBC_FLOOR, GLIBCXX $GLIBCXX_FLOOR, CXXABI $CXXABI_FLOOR"
# the largest numbered version of one family, e.g. max_of GLIBCXX < versions
max_of() { sed -n "s/^$1_\([0-9][0-9.]*\)\$/\1/p" | sort -V | tail -1; }
# a <= b, as sort -V orders them
ver_le() { [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" = "$2" ]; }
if [ "$dry" = 0 ]; then
	[ -x "$bin" ] || { echo "$me: the build finished but $bin is not there" >&2; exit 1; }
	[ -d "$data" ] || { echo "$me: the build finished but $data is not there" >&2; exit 1; }
	bad=0
	elf=$("$TOOLCHAIN/bin/aarch64-linux-gnu-readelf" -hl "$bin")
	case $elf in *"Machine:"*"AArch64"*) ;; *) echo "$me: $bin is not an AArch64 program" >&2; bad=1 ;; esac
	case $elf in
	*"interpreter: /lib/ld-linux-aarch64.so.1]"*) ;;
	*) echo "$me: $bin does not use /lib/ld-linux-aarch64.so.1" >&2; bad=1 ;;
	esac
	needed=$("$TOOLCHAIN/bin/aarch64-linux-gnu-objdump" -p "$bin" | sed -n 's/^ *NEEDED *//p' | paste -sd' ' -)
	case " $needed " in
	*" libGLESv2."* | *" libEGL."* | *" libGL."*)
		echo "$me: $bin links a GL or EGL library (NEEDED: $needed); it must dlopen them" >&2
		bad=1 ;;
	esac
	versions=$("$TOOLCHAIN/bin/aarch64-linux-gnu-objdump" -T "$bin" |
		{ grep -oE '(GLIBCXX|GLIBC|CXXABI)_[0-9A-Z][0-9A-Z.]*' || true; } | sort -u)
	case $versions in
	*_PRIVATE*) echo "$me: $bin needs a private glibc symbol version" >&2; bad=1 ;;
	esac
	glibc_max=$(printf '%s\n' "$versions" | max_of GLIBC)
	glibcxx_max=$(printf '%s\n' "$versions" | max_of GLIBCXX)
	cxxabi_max=$(printf '%s\n' "$versions" | max_of CXXABI)
	[ -n "$glibc_max" ] || { echo "$me: objdump -T shows no GLIBC version in $bin" >&2; bad=1; }
	ver_le "${glibc_max:-0}" "$GLIBC_FLOOR" || { echo "$me: $bin needs GLIBC_$glibc_max > $GLIBC_FLOOR" >&2; bad=1; }
	ver_le "${glibcxx_max:-0}" "$GLIBCXX_FLOOR" || { echo "$me: $bin needs GLIBCXX_$glibcxx_max > $GLIBCXX_FLOOR" >&2; bad=1; }
	ver_le "${cxxabi_max:-0}" "$CXXABI_FLOOR" || { echo "$me: $bin needs CXXABI_$cxxabi_max > $CXXABI_FLOOR" >&2; bad=1; }
	[ "$bad" = 0 ] || { echo "$me: $bin does not fit the board; nothing written to $out_abs" >&2; exit 1; }
	echo "$me: GLIBC_$glibc_max GLIBCXX_${glibcxx_max:-none} CXXABI_${cxxabi_max:-none}: within the floors"
	echo "$me: NEEDED $needed"
fi

# --- 5. the build directory -----------------------------------------------------
step "5. write $out_abs (through $part)"
run rm -rf "$part"
run mkdir -p "$part/data"
run cp "$bin" "$part/deqp-gles2"
run cp -r "$data" "$part/data/gles2"
run cp "$MUSTPASS" "$part/mustpass"
if [ -n "$gles_lib" ]; then
	run mkdir -p "$part/lib"
	run cp "$gles_lib/libGLESv2.so.2" "$gles_lib/libEGL.so.1" "$part/lib/"
fi
if [ "$dry" = 1 ]; then
	echo "+ write $part/build.txt (commit=$commit dirty=$dirty cts=$CTS_COMMIT toolchain= glibc_max= glibcxx_max= cxxabi_max= build_seconds=)"
else
	{
		echo "# tools/cts/mali-deqp-build.sh: deqp-gles2 for the NanoPC-T6's Armbian trixie"
		echo "commit=$commit"
		if [ "$dirty" = yes ]; then
			echo "dirty=yes (--allow-dirty: built from a tree with changes, a local test, not a record)"
		else
			echo "dirty=no"
		fi
		echo "cts=$CTS_COMMIT"
		echo "cts_patch=cts-yakogl-target.patch sha256 $patch_sum"
		echo "toolchain=$toolchain, Debian trixie MALI_DEQP_TOOLCHAIN (tools/cts/deps.toml)"
		echo "glibc_max=$glibc_max"
		echo "glibcxx_max=${glibcxx_max:-none}"
		echo "cxxabi_max=${cxxabi_max:-none}"
		echo "floors=GLIBC_$GLIBC_FLOOR GLIBCXX_$GLIBCXX_FLOOR CXXABI_$CXXABI_FLOOR"
		echo "build_seconds=$build_seconds"
		echo "deqp_gles2_bytes=$(stat -c %s "$bin")"
	} > "$part/build.txt"
fi
[ -d "$out_abs" ] && run rmdir "$out_abs"
run mv "$part" "$out_abs"
if [ "$dry" = 0 ]; then
	echo "$me: $out_abs: deqp-gles2 $(stat -c %s "$out_abs/deqp-gles2") bytes, build ${build_seconds}s, commit $commit${gles_lib:+, lib/ from $gles_lib}"
fi
