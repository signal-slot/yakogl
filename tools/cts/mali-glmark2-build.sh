#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build glmark2-es2 for the NanoPC-T6's Armbian (Debian trixie, aarch64; Mali-G610
# over panthor) on YakoGL's Mali pair, into the tree tools/cts/mali-glmark2-run.sh runs.
#
#   tools/cts/mali-glmark2-build.sh [--dev] [--dry-run] [--gles-lib DIR] <outdir>
#   tools/cts/mali-glmark2-build.sh --check <outdir>
#
# The program is glmark2 at GLMARK2_PIN (the pin of tools/hostharness/glmark2/
# run.sh and tools/setup-gate-host.sh) with mali-glmark2-pbuffer.patch: a new
# flavour, pbuffer-glesv2, whose native system is an 800x600 (glmark2's
# --size) EGL pbuffer surface on EGL_DEFAULT_DISPLAY. The pair
# (tools/mali/panthor-gles) offers pbuffer surfaces only, and glmark2's own
# flavours all need a window: x11 and wayland a window system, drm and gbm a
# GBM device, null a surfaceless context on EGL_MESA_platform_surfaceless.
# The patch also prints, after each scene that ran, the picture line of
# driver/docs/glmark2.md "The picture value" on stderr:
#   glmark2-picture: <scene>: 800x600; probes c ..., tl ..., tr ..., bl ..., br ...;
#     crc16 XXXX, crc16q XXXX; mean R G B; black yes|no             (one line)
# read with glReadPixels at the scene's last frame, before its swap (the code
# of driver/samples/zephyr/glmark2/src/main.cpp, capture_picture), and finds
# its data beside itself (<outdir>/share/glmark2; --data-path still wins).
# With GLMARK2_FB=1 in the environment it also prints each scene's frame on
# stderr, the 'glmark2-fb:' dump of driver/docs/glmark2.md "The frame dump"
# (the format of glmark2-host-ref.patch; tools/cts/glmark2-fb.py decodes it,
# one scene per process: about 1.9 MB of text per scene at 800x600). Unset, or
# any other value, the output is the picture line alone.
#
# glmark2 links no GL or EGL library: it dlopen()s libEGL.so.1 and
# libGLESv2.so.2, which its RUNPATH $ORIGIN/../lib finds in <outdir>/lib
# before the board's glvnd + Mesa 25.0.7 (the same two SONAMEs: the "sonama
# trap" of #327's probe, docs/es2/board-recipe-mali.md). libpng, libjpeg and
# zlib are linked statically from pinned Debian trixie arm64 -dev packages
# (IMG_PKGS), so the program needs only libc, libm, libstdc++ and libgcc_s.
#
# Output, <outdir> absent or empty beforehand (written as <outdir>.part, renamed
# at the end):
#   <outdir>/bin/glmark2-es2-mali       the program (RUNPATH $ORIGIN/../lib)
#   <outdir>/lib/libGLESv2.so.2, libEGL.so.1
#                                       the pair: tools/mali/panthor-gles/build.sh
#                                       (default), or copied from --gles-lib DIR
#   <outdir>/share/glmark2/             glmark2's data (shaders, models, textures)
#   <outdir>/BUILD-INFO                 key=value lines: commit=<git describe of
#                                       yakogl>, dev=, glmark2=<pin>, glmark2_patch=,
#                                       flavour=pbuffer, toolchain=, glibc_max=,
#                                       glibcxx_max=, cxxabi_max= (the largest
#                                       version the program and the pair need),
#                                       floors=
#
# The board's ABI floors (Armbian trixie): GLIBC 2.41, GLIBCXX 3.4.33,
# CXXABI 1.3.15. The compiler is Debian trixie's own g++-14-aarch64-linux-gnu,
# MALI_DEQP_TOOLCHAIN of tools/cts/deps.toml, as mali-deqp-build.sh uses it.
#
# The commit: HEAD must be an ancestor of the main ref and the working tree
# clean (no change to a tracked file), so BUILD-INFO's commit is main code;
# else the build is refused with exit 3. --dev builds anyway, for local tests
# only: BUILD-INFO then says dev=yes and the tree is not a record.
#
# --dry-run checks the arguments and the commit and prints every step, running
# none and writing nothing (no toolchain, glmark2 or network is needed).
#
# --check <outdir> (host, readelf only): the program is an AArch64 executable
# on /lib/ld-linux-aarch64.so.1 with RUNPATH $ORIGIN/../lib and no RPATH; it
# NEEDs nothing beyond libc, libm, libstdc++, libgcc_s and the loader (no
# libEGL, libGLESv2, libGL, glvnd, libpng, ...); lib/ holds exactly the pair
# (SONAMEs, libEGL.so.1 needing libGLESv2.so.2 through RUNPATH $ORIGIN,
# libGLESv2.so.2 carrying the "yakogl-mali: loaded" contract text: not Mesa's)
# and no other libEGL/libGLESv2 is anywhere in <outdir>; the program and the
# pair need no GLIBC, GLIBCXX or CXXABI version above the floors (readelf -V);
# share/glmark2 has glmark2's data; BUILD-INFO has every key, the pin, flavour
# pbuffer and the maxima readelf finds. It prints the maxima.
#
# Exit: 0 done (or every check passed), 1 a step or a check failed, 2 usage or
# a missing tool, 3 refused (HEAD not on the main ref, or a dirty tree).
#
# Environment:
#   GLMARK2_SRC   glmark2 at GLMARK2_PIN: a git checkout without local changes,
#                 or the GitHub archive directory glmark2-<pin>; never written
#                 to (default <deps>/glmark2-src; tools/cts/deps-env.sh --build
#                 GLMARK2_SRC makes it)
#   TOOLCHAIN     MALI_DEQP_TOOLCHAIN (default <deps>/debian-trixie-arm64-cross;
#                 made with tools/cts/deps-env.sh --build --board nanopc_t6
#                 MALI_DEQP_TOOLCHAIN when the default is absent)
#   IMG_SYSROOT   the unpacked IMG_PKGS (default <deps>/mali-glmark2-imglibs,
#                 fetched from snapshot.debian.org, sha256-checked)
#   BUILD         build tree (default <repo>/build/cts/mali-glmark2)
#   JOBS          parallel compile jobs (default nproc)
#   MALI_GLMARK2_MAIN_REF   the main ref HEAD must be on (default origin/main)
#   MALI_GLMARK2_REPO       the yakogl checkout built (default this script's)
#   READELF       readelf for --check (default the toolchain's, else readelf)
# <deps> is the dependency layout, tools/cts/deps-env.sh --layout.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
me=mali-glmark2-build
GLMARK2_PIN=22c527cb0556f3a1ac4445aaa52cc532760928d5
GLIBC_FLOOR=2.41
GLIBCXX_FLOOR=3.4.33
CXXABI_FLOOR=1.3.15
PATCH=$HERE/mali-glmark2-pbuffer.patch
PAIR=$(cd "$HERE/../mali/panthor-gles" && pwd)
FLAVOUR=pbuffer
PROGRAM=glmark2-es2-mali
# What the program may need: the board's libc, libm, libstdc++, libgcc_s and loader.
NEEDED_OK="libc.so.6 libm.so.6 libstdc++.so.6 libgcc_s.so.1 ld-linux-aarch64.so.1"
# Debian trixie arm64 (snapshot 20261101T000000Z): name version pool path sha256.
# Their static archives (libpng16.a, libjpeg.a, libz.a) and headers only.
IMG_PKGS="libpng-dev 1.6.48-1+deb13u5 pool/main/libp/libpng1.6/libpng-dev_1.6.48-1+deb13u5_arm64.deb a212fde02b2930302edcc959eaa2079bb459def3a92741cc09f6f55c250b8ca0
libjpeg62-turbo-dev 1:2.1.5-4 pool/main/libj/libjpeg-turbo/libjpeg62-turbo-dev_2.1.5-4_arm64.deb bdd0dd56c1b33de4d19d761dfbd69729e813a57ef008a8dd3e0380a3e1b11163
zlib1g-dev 1:1.3.dfsg+really1.3.1-1+b1 pool/main/z/zlib/zlib1g-dev_1.3.dfsg+really1.3.1-1+b1_arm64.deb 745ebcf1fa115e230af76216355d7290a24f1790fe3780249b519c0e4d30534f"
IMG_MIRRORS="https://snapshot.debian.org/archive/debian/20261101T000000Z https://deb.debian.org/debian"

usage() {
	sed -n '8,9p' "$0" | sed 's/^# \{0,1\}//' >&2
	exit 2
}

out='' gles_lib='' dry=0 dev=0 check=0
while [ $# -gt 0 ]; do
	case $1 in
	--dev) dev=1; shift ;;
	--dry-run) dry=1; shift ;;
	--gles-lib) gles_lib=${2:?--gles-lib needs a directory}; shift 2 ;;
	--check) check=1; out=${2:?--check needs an outdir}; shift 2 ;;
	-h | --help) sed -n '5,88p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	-*) echo "$me: unknown argument '$1'" >&2; usage ;;
	*) [ -z "$out" ] || { echo "$me: one <outdir> only" >&2; usage; }; out=$1; shift ;;
	esac
done
[ -n "$out" ] || usage

# --- the floors and the tree check (--check, and the build's last step) -------
# the largest numbered version of one family, e.g. max_of GLIBCXX < versions
max_of() { sed -n "s/^$1_\([0-9][0-9.]*\)\$/\1/p" | sort -V | tail -1; }
# a <= b, as sort -V orders them
ver_le() { [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" = "$2" ]; }

bad=0
fail() { echo "$me: --check: $*" >&2; bad=1; }
# the bracketed values of one readelf -d tag, space-separated
dyn() { sed -n "s/^ *0x[0-9a-f]* *($2) *[^[]*\[\(.*\)\]\$/\1/p" <<< "$1" | paste -sd' ' -; }

# check_tree <dir> [info]: every check above; with "info" BUILD-INFO too.
# Sets glibc_max, glibcxx_max, cxxabi_max (over the program and the pair).
check_tree() {
	local d=$1 with_info=${2:-} bin versions="" f re needed n
	bad=0
	bin=$d/bin/$PROGRAM
	command -v "$READELF" > /dev/null 2>&1 || { echo "$me: --check needs readelf ($READELF)" >&2; exit 2; }
	for f in "$bin" "$d/lib/libGLESv2.so.2" "$d/lib/libEGL.so.1"; do
		[ -f "$f" ] || { fail "no ${f#"$d"/}"; return 0; }
	done
	# the program
	re=$("$READELF" -hldVW "$bin" 2>&1) || { fail "readelf cannot read bin/$PROGRAM"; return 0; }
	case $re in *"Machine:"*"AArch64"*) ;; *) fail "bin/$PROGRAM is not AArch64" ;; esac
	case $re in *"Type:"*"EXEC"* | *"Type:"*"DYN"*) ;; *) fail "bin/$PROGRAM is not an executable" ;; esac
	case $re in
	*"interpreter: /lib/ld-linux-aarch64.so.1]"*) ;;
	*) fail "bin/$PROGRAM does not use /lib/ld-linux-aarch64.so.1" ;;
	esac
	[ "$(dyn "$re" RUNPATH)" = "\$ORIGIN/../lib" ] ||
		fail "bin/$PROGRAM has RUNPATH '$(dyn "$re" RUNPATH)', not \$ORIGIN/../lib (the pair would not be found first)"
	[ -z "$(dyn "$re" RPATH)" ] || fail "bin/$PROGRAM has an RPATH ($(dyn "$re" RPATH)); only RUNPATH \$ORIGIN/../lib"
	needed=$(dyn "$re" NEEDED)
	for n in $needed; do
		case $n in
		libEGL.* | libGLESv2.* | libGLESv1* | libGL.* | libGLX* | libGLdispatch* | libOpenGL* | libgbm* | libwayland* | libX11*)
			fail "bin/$PROGRAM NEEDs $n: a system GL/EGL library (glmark2 must dlopen the pair's)" ;;
		*) case " $NEEDED_OK " in
			*" $n "*) ;;
			*) fail "bin/$PROGRAM NEEDs $n, which a minimal Armbian trixie may lack (allowed: $NEEDED_OK)" ;;
			esac ;;
		esac
	done
	versions+=$re$'\n'
	# the pair
	re=$("$READELF" -hldVW "$d/lib/libGLESv2.so.2" 2>&1) || { fail "readelf cannot read lib/libGLESv2.so.2"; return 0; }
	case $re in *"Machine:"*"AArch64"*) ;; *) fail "lib/libGLESv2.so.2 is not AArch64" ;; esac
	[ "$(dyn "$re" SONAME)" = libGLESv2.so.2 ] || fail "lib/libGLESv2.so.2 has SONAME '$(dyn "$re" SONAME)'"
	case " $(dyn "$re" NEEDED) " in
	*" libEGL."* | *" libGLESv2."* | *" libGL."* | *" libGLdispatch"*)
		fail "lib/libGLESv2.so.2 NEEDs a GL library ($(dyn "$re" NEEDED)): not the pair" ;;
	esac
	grep -aq 'yakogl-mali: loaded' "$d/lib/libGLESv2.so.2" ||
		fail "lib/libGLESv2.so.2 has no 'yakogl-mali: loaded' text: not the panthor-gles pair (the board's Mesa has the same SONAME)"
	versions+=$re$'\n'
	re=$("$READELF" -hldVW "$d/lib/libEGL.so.1" 2>&1) || { fail "readelf cannot read lib/libEGL.so.1"; return 0; }
	case $re in *"Machine:"*"AArch64"*) ;; *) fail "lib/libEGL.so.1 is not AArch64" ;; esac
	[ "$(dyn "$re" SONAME)" = libEGL.so.1 ] || fail "lib/libEGL.so.1 has SONAME '$(dyn "$re" SONAME)'"
	case " $(dyn "$re" NEEDED) " in
	*" libGLESv2.so.2 "*) ;;
	*) fail "lib/libEGL.so.1 does not NEED libGLESv2.so.2 ($(dyn "$re" NEEDED)): not the pair's forwarder" ;;
	esac
	[ "$(dyn "$re" RUNPATH)" = "\$ORIGIN" ] || fail "lib/libEGL.so.1 has RUNPATH '$(dyn "$re" RUNPATH)', not \$ORIGIN"
	versions+=$re$'\n'
	# no other libEGL/libGLESv2 anywhere in the tree
	while IFS= read -r f; do
		case $f in
		"$d/lib/libGLESv2.so.2" | "$d/lib/libEGL.so.1") ;;
		*) fail "${f#"$d"/}: a libEGL/libGLESv2 other than the pair's" ;;
		esac
	done < <(find "$d" \( -name 'libEGL*' -o -name 'libGLESv2*' -o -name 'libGL.so*' \) \( -type f -o -type l \))
	# the floors
	versions=$(grep -oE 'Name: (GLIBCXX|GLIBC|CXXABI)_[0-9A-Z][0-9A-Z.]*' <<< "$versions" | sed 's/^Name: //' | sort -u || true)
	case $versions in *_PRIVATE*) fail "a private glibc symbol version is needed" ;; esac
	glibc_max=$(max_of GLIBC <<< "$versions")
	glibcxx_max=$(max_of GLIBCXX <<< "$versions")
	cxxabi_max=$(max_of CXXABI <<< "$versions")
	[ -n "$glibc_max" ] || fail "readelf -V shows no GLIBC version need"
	ver_le "${glibc_max:-0}" "$GLIBC_FLOOR" || fail "GLIBC_$glibc_max needed > the board's $GLIBC_FLOOR"
	ver_le "${glibcxx_max:-0}" "$GLIBCXX_FLOOR" || fail "GLIBCXX_$glibcxx_max needed > the board's $GLIBCXX_FLOOR"
	ver_le "${cxxabi_max:-0}" "$CXXABI_FLOOR" || fail "CXXABI_$cxxabi_max needed > the board's $CXXABI_FLOOR"
	glibcxx_max=${glibcxx_max:-none} cxxabi_max=${cxxabi_max:-none}
	# the data
	for f in shaders models textures; do
		[ -n "$(ls -A "$d/share/glmark2/$f" 2>/dev/null)" ] || fail "share/glmark2/$f is missing or empty"
	done
	[ "$with_info" = info ] || return 0
	# BUILD-INFO
	local info=$d/BUILD-INFO k
	[ -f "$info" ] || { fail "no BUILD-INFO"; return 0; }
	for k in commit dev glmark2 flavour glibc_max glibcxx_max cxxabi_max; do
		grep -q "^$k=." "$info" || fail "BUILD-INFO has no $k="
	done
	[ "$(info_val "$info" glmark2)" = "$GLMARK2_PIN" ] ||
		fail "BUILD-INFO glmark2=$(info_val "$info" glmark2), not the pin $GLMARK2_PIN"
	[ "$(info_val "$info" flavour)" = "$FLAVOUR" ] ||
		fail "BUILD-INFO flavour=$(info_val "$info" flavour), not $FLAVOUR"
	for k in glibc_max glibcxx_max cxxabi_max; do
		[ "$(info_val "$info" "$k")" = "${!k}" ] ||
			fail "BUILD-INFO $k=$(info_val "$info" "$k"), but readelf finds ${!k}"
	done
}
# info_val <BUILD-INFO> <key>: the key's value (its first line)
info_val() { sed -n "s/^$2=//p" "$1" | head -1; }

if [ "$check" = 1 ]; then
	if [ "$dry" = 1 ] || [ -n "$gles_lib" ]; then
		echo "$me: --check takes only <outdir>" >&2
		usage
	fi
	[ -d "$out" ] || { echo "$me: --check: no directory $out" >&2; exit 1; }
	tc=${TOOLCHAIN:-$("$HERE/deps-env.sh" --layout)/debian-trixie-arm64-cross}
	if [ -z "${READELF:-}" ]; then
		READELF=readelf
		[ -x "$tc/bin/aarch64-linux-gnu-readelf" ] && READELF=$tc/bin/aarch64-linux-gnu-readelf
	fi
	check_tree "$out" info
	if [ "$bad" != 0 ]; then
		echo "$me: --check: $out does not fit the board" >&2
		exit 1
	fi
	echo "$me: --check: $out: GLIBC_$glibc_max GLIBCXX_$glibcxx_max CXXABI_$cxxabi_max (floors GLIBC_$GLIBC_FLOOR GLIBCXX_$GLIBCXX_FLOOR CXXABI_$CXXABI_FLOOR); bin/$PROGRAM RUNPATH \$ORIGIN/../lib; lib/ is the pair; commit $(sed -n 's/^commit=//p' "$out/BUILD-INFO")"
	exit 0
fi

# --- 0. the arguments and the commit ------------------------------------------
REPO=${MALI_GLMARK2_REPO:-$(cd "$HERE/../.." && pwd)}
MAIN_REF=${MALI_GLMARK2_MAIN_REF:-origin/main}
if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
	echo "$me: $out exists and is not empty (a build tree is never overwritten)" >&2
	exit 1
fi
if [ -n "$gles_lib" ]; then
	for l in libGLESv2.so.2 libEGL.so.1; do
		[ -f "$gles_lib/$l" ] || { echo "$me: --gles-lib $gles_lib has no $l" >&2; exit 1; }
	done
	gles_lib=$(cd "$gles_lib" && pwd)
fi
head=$(git -C "$REPO" rev-parse --verify HEAD)
refuse=
if ! git -C "$REPO" rev-parse --verify -q "$MAIN_REF^{commit}" > /dev/null; then
	refuse="the main ref $MAIN_REF does not resolve in $REPO (git fetch origin, or MALI_GLMARK2_MAIN_REF=<ref>)"
elif ! git -C "$REPO" merge-base --is-ancestor "$head" "$MAIN_REF"; then
	refuse="HEAD $head is not on $MAIN_REF: a record is built from one main commit"
elif [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no)" ]; then
	refuse="the working tree has changes to tracked files: BUILD-INFO's commit would not be the code's"
fi
if [ -n "$refuse" ]; then
	if [ "$dev" = 1 ]; then
		echo "$me: $refuse (--dev: a local test build, not a record)" >&2
	else
		echo "$me: refused: $refuse. --dev builds anyway, for local tests only." >&2
		exit 3
	fi
fi
describe=$(git -C "$REPO" describe --always --abbrev=40 --dirty)
dev_info=no
[ "$dev" = 0 ] || dev_info="yes (--dev: a local test build, not a record${refuse:+; $refuse})"

deps=$("$HERE/deps-env.sh" --layout)
GLMARK2_SRC=${GLMARK2_SRC:-$deps/glmark2-src}
toolchain_default=$deps/debian-trixie-arm64-cross
TOOLCHAIN=${TOOLCHAIN:-$toolchain_default}
IMG_SYSROOT=${IMG_SYSROOT:-$deps/mali-glmark2-imglibs}
BUILD=${BUILD:-$REPO/build/cts/mali-glmark2}
JOBS=${JOBS:-$(nproc)}
out_parent=$(dirname "$out")
out_abs=$( [ -d "$out_parent" ] && cd "$out_parent" && pwd || echo "$out_parent")/$(basename "$out")
part=$out_abs.part
patch_sum=$(sha256sum "$PATCH" | cut -c1-64)

# A step: printed always, run unless --dry-run.
run() {
	printf '+'
	printf ' %q' "$@"
	printf '\n'
	[ "$dry" = 1 ] || "$@"
}
step() { printf '== %s\n' "$*"; }

[ "$dry" = 0 ] || echo "$me: --dry-run: the steps below are printed, none is run"
echo "$me: commit=$describe dev=${dev_info%% *} glmark2=$GLMARK2_PIN flavour=$FLAVOUR"
echo "$me: GLMARK2_SRC=$GLMARK2_SRC TOOLCHAIN=$TOOLCHAIN IMG_SYSROOT=$IMG_SYSROOT BUILD=$BUILD out=$out_abs"

# --- 1. the toolchain -----------------------------------------------------------
gcc=$TOOLCHAIN/bin/aarch64-linux-gnu-gcc
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
toolchain=
if [ "$dry" = 0 ]; then
	[ -x "$gxx" ] || { echo "$me: no $gxx after the toolchain step" >&2; exit 1; }
	toolchain=$("$gxx" --version | sed -n 1p)
	echo "$me: $toolchain"
	for t in meson ninja pkg-config; do
		command -v "$t" > /dev/null 2>&1 || { echo "$me: needs $t on PATH" >&2; exit 2; }
	done
fi

# --- 2. libpng, libjpeg and zlib for a static link --------------------------------
# The -dev packages only: their .a and headers. The shared-object links they
# carry are removed, so the link can only take the static archives.
step "2. the static image libraries (IMG_PKGS: libpng-dev, libjpeg62-turbo-dev, zlib1g-dev, Debian trixie arm64) in $IMG_SYSROOT"
img_stamp=$(printf '%s\n' "$IMG_PKGS" | sha256sum | cut -c1-16)
fetch() { # url file
	if command -v curl > /dev/null 2>&1; then
		curl -fsSL --retry 2 -o "$2" "$1"
	else
		python3 -I -c 'import sys, urllib.request as u; open(sys.argv[2], "wb").write(u.urlopen(sys.argv[1], timeout=300).read())' "$1" "$2"
	fi
}
make_img_sysroot() { # dir
	local dir=$1 p=$1.part name ver path sum deb got m member z
	rm -rf "$p"
	mkdir -p "$p/.debs"
	while read -r name ver path sum; do
		deb=$p/.debs/${path##*/}
		got=
		for m in $IMG_MIRRORS; do
			if fetch "$m/$path" "$deb" 2> /dev/null && [ "$(sha256sum "$deb" | cut -c1-64)" = "$sum" ]; then
				got=1
				break
			fi
		done
		[ -n "$got" ] || { echo "$me: could not fetch $name $ver ($path) with sha256 $sum" >&2; return 1; }
		member=$(ar t "$deb" | grep '^data\.tar')
		case $member in *.xz) z=-J ;; *.zst) z=--zstd ;; *.gz) z=-z ;; *) z= ;; esac
		ar p "$deb" "$member" | tar -x $z -f - -C "$p"
	done <<< "$IMG_PKGS"
	find "$p/usr/lib" -name '*.so' -delete
	rm -rf "$p/.debs" "$p/usr/share/doc"
	for a in libpng16.a libjpeg.a libz.a; do
		[ -f "$p/usr/lib/aarch64-linux-gnu/$a" ] || { echo "$me: the packages have no usr/lib/aarch64-linux-gnu/$a" >&2; return 1; }
	done
	echo "$img_stamp" > "$p/.yakogl-imglibs"
	rm -rf "$dir"
	mv "$p" "$dir"
}
if [ "$(cat "$IMG_SYSROOT/.yakogl-imglibs" 2> /dev/null)" = "$img_stamp" ]; then
	echo "present: $IMG_SYSROOT"
elif [ "$dry" = 1 ]; then
	printf '%s\n' "$IMG_PKGS" | while read -r name ver path sum; do
		echo "+ fetch ${IMG_MIRRORS%% *}/$path (sha256 $sum), unpack its data into $IMG_SYSROOT"
	done
	echo "+ remove the .so links of $IMG_SYSROOT/usr/lib, keep the .a and the headers"
else
	make_img_sysroot "$IMG_SYSROOT"
fi

# --- 3. the pair ------------------------------------------------------------------
gles_out=$BUILD/gles
if [ -n "$gles_lib" ]; then
	step "3. the pair: libGLESv2.so.2 and libEGL.so.1 from --gles-lib $gles_lib"
	gles_out=$gles_lib
else
	step "3. the pair: tools/mali/panthor-gles/build.sh $gles_out (release, ES 2.0)"
	run "$PAIR/build.sh" "$gles_out"
fi
run "$PAIR/check.sh" "$gles_out"

# --- 4. a private glmark2 copy with the pbuffer patch -------------------------------
# GLMARK2_SRC is never written to: a --shared clone of the pin (or a copy of
# the archive) goes to $BUILD/glmark2 and the patch is applied there. The
# copy is made again when the patch changes.
step "4. glmark2 $GLMARK2_PIN with mali-glmark2-pbuffer.patch in $BUILD/glmark2"
src_kind=
if [ -d "$GLMARK2_SRC" ]; then
	src_phys=$(cd "$GLMARK2_SRC" && pwd -P)
	if [ "$(git -C "$src_phys" rev-parse --show-toplevel 2> /dev/null)" = "$src_phys" ]; then
		[ "$(git -C "$src_phys" rev-parse HEAD)" = "$GLMARK2_PIN" ] &&
			[ -z "$(git -C "$src_phys" status --porcelain --untracked-files=no)" ] && src_kind=git
	elif [ "$(basename "$src_phys")" = "glmark2-$GLMARK2_PIN" ] && [ -f "$src_phys/src/default-benchmarks.h" ]; then
		src_kind=archive
	fi
fi
if [ -z "$src_kind" ]; then
	msg="GLMARK2_SRC=$GLMARK2_SRC is no glmark2 tree at the pin $GLMARK2_PIN (a git checkout of it without local changes, or the archive directory glmark2-$GLMARK2_PIN): tools/cts/deps-env.sh --build GLMARK2_SRC"
	[ "$dry" = 1 ] || { echo "$me: $msg" >&2; exit 1; }
	echo "(not present: $msg)"
	src_kind=git
fi
if [ "$(cat "$BUILD/glmark2/.yakogl-patch" 2> /dev/null)" != "$patch_sum" ]; then
	run rm -rf "$BUILD/glmark2" "$BUILD/meson"
	run mkdir -p "$BUILD"
	if [ "$src_kind" = git ]; then
		run git clone -q --shared --no-checkout "$GLMARK2_SRC" "$BUILD/glmark2"
		run git -C "$BUILD/glmark2" checkout -q --detach "$GLMARK2_PIN"
		run git -C "$BUILD/glmark2" apply "$PATCH"
	else
		run cp -a "$GLMARK2_SRC" "$BUILD/glmark2"
		run git apply --directory="$BUILD/glmark2" --unsafe-paths "$PATCH"
	fi
	[ "$dry" = 1 ] || echo "$patch_sum" > "$BUILD/glmark2/.yakogl-patch"
fi

# --- 5. configure and build -----------------------------------------------------------
# The cross file: trixie's drivers (they pass --sysroot themselves), pkg-config
# only inside IMG_SYSROOT (its .pc files, sysroot-prefixed), static libraries,
# RUNPATH $ORIGIN/../lib. Only the pbuffer-glesv2 program is built.
step "5. configure and build glmark2-es2-pbuffer in $BUILD/meson"
xfile=$BUILD/cross-trixie-aarch64.ini
img_lib=$IMG_SYSROOT/usr/lib/aarch64-linux-gnu
if [ "$dry" = 1 ]; then
	echo "+ write $xfile (aarch64 linux, $TOOLCHAIN/bin/aarch64-linux-gnu-{gcc,g++,ar,strip}, pkg-config in $IMG_SYSROOT, RUNPATH \$ORIGIN/../lib)"
else
	mkdir -p "$BUILD"
	# written only when it changes: a new file makes meson configure again
	{
		echo "# Generated by tools/cts/mali-glmark2-build.sh: Debian trixie's arm64 cross toolchain."
		echo "[binaries]"
		echo "c = '$gcc'"
		echo "cpp = '$gxx'"
		echo "ar = '$TOOLCHAIN/bin/aarch64-linux-gnu-ar'"
		echo "strip = '$TOOLCHAIN/bin/aarch64-linux-gnu-strip'"
		echo "pkg-config = '$(command -v pkg-config)'"
		echo "[properties]"
		echo "pkg_config_libdir = ['$img_lib/pkgconfig', '$IMG_SYSROOT/usr/share/pkgconfig']"
		echo "sys_root = '$IMG_SYSROOT'"
		echo "[built-in options]"
		echo "c_args = ['-isystem', '$IMG_SYSROOT/usr/include/aarch64-linux-gnu']"
		echo "cpp_args = ['-isystem', '$IMG_SYSROOT/usr/include/aarch64-linux-gnu']"
		echo "cpp_link_args = ['-L$img_lib', '-Wl,--enable-new-dtags', '-Wl,-rpath,\$ORIGIN/../lib', '-Wl,--as-needed', '-Wl,--build-id']"
		echo "[host_machine]"
		echo "system = 'linux'"
		echo "cpu_family = 'aarch64'"
		echo "cpu = 'cortex-a76'"
		echo "endian = 'little'"
	} > "$xfile.new"
	if cmp -s "$xfile.new" "$xfile"; then rm "$xfile.new"; else mv "$xfile.new" "$xfile"; fi
fi
if [ ! -f "$BUILD/meson/build.ninja" ] || [ "$xfile" -nt "$BUILD/meson/build.ninja" ]; then
	run rm -rf "$BUILD/meson"
	run meson setup "$BUILD/meson" "$BUILD/glmark2" --cross-file "$xfile" \
		--buildtype=release -Dprefer_static=true -Dflavors=pbuffer-glesv2
fi
t0=$(date +%s)
run ninja -C "$BUILD/meson" -j"$JOBS" src/glmark2-es2-pbuffer
build_seconds=$(( $(date +%s) - t0 ))
bin=$BUILD/meson/src/glmark2-es2-pbuffer

# --- 6. the tree ------------------------------------------------------------------------
step "6. write $out_abs (through $part), check it, rename it"
run rm -rf "$part"
run mkdir -p "$part/bin" "$part/lib" "$part/share/glmark2"
run cp "$bin" "$part/bin/$PROGRAM"
run cp "$gles_out/libGLESv2.so.2" "$gles_out/libEGL.so.1" "$part/lib/"
run cp -r "$BUILD/glmark2/data/shaders" "$BUILD/glmark2/data/models" "$BUILD/glmark2/data/textures" "$part/share/glmark2/"
if [ "$dry" = 1 ]; then
	echo "+ check $part (readelf: AArch64, RUNPATH \$ORIGIN/../lib, NEEDED within $NEEDED_OK, the pair, GLIBC_$GLIBC_FLOOR GLIBCXX_$GLIBCXX_FLOOR CXXABI_$CXXABI_FLOOR)"
	echo "+ write $part/BUILD-INFO (commit=$describe dev=${dev_info%% *} glmark2=$GLMARK2_PIN flavour=$FLAVOUR glibc_max= glibcxx_max= cxxabi_max=)"
	echo "+ check $part with its BUILD-INFO (--check)"
else
	if [ -z "${READELF:-}" ]; then
		READELF=$TOOLCHAIN/bin/aarch64-linux-gnu-readelf
	fi
	check_tree "$part"
	[ "$bad" = 0 ] || { echo "$me: $part does not fit the board; $out_abs not written" >&2; exit 1; }
	{
		echo "# tools/cts/mali-glmark2-build.sh: glmark2-es2 ($FLAVOUR) for the NanoPC-T6's Armbian trixie"
		echo "commit=$describe"
		echo "dev=$dev_info"
		echo "glmark2=$GLMARK2_PIN"
		echo "glmark2_patch=mali-glmark2-pbuffer.patch sha256 $patch_sum"
		echo "flavour=$FLAVOUR"
		echo "toolchain=$toolchain, Debian trixie MALI_DEQP_TOOLCHAIN (tools/cts/deps.toml)"
		echo "gles_lib=${gles_lib:-tools/mali/panthor-gles/build.sh}"
		echo "glibc_max=$glibc_max"
		echo "glibcxx_max=$glibcxx_max"
		echo "cxxabi_max=$cxxabi_max"
		echo "floors=GLIBC_$GLIBC_FLOOR GLIBCXX_$GLIBCXX_FLOOR CXXABI_$CXXABI_FLOOR"
		echo "build_seconds=$build_seconds"
	} > "$part/BUILD-INFO"
	check_tree "$part" info
	[ "$bad" = 0 ] || { echo "$me: $part/BUILD-INFO does not check; $out_abs not written" >&2; exit 1; }
fi
[ -d "$out_abs" ] && run rmdir "$out_abs"
run mv "$part" "$out_abs"
if [ "$dry" = 0 ]; then
	echo "$me: $out_abs: bin/$PROGRAM $(stat -c %s "$out_abs/bin/$PROGRAM") bytes, GLIBC_$glibc_max GLIBCXX_$glibcxx_max CXXABI_$cxxabi_max (floors GLIBC_$GLIBC_FLOOR GLIBCXX_$GLIBCXX_FLOOR CXXABI_$CXXABI_FLOOR), commit $describe"
fi
