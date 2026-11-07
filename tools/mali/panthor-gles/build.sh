#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build libGLESv2.so.2 and libEGL.so.1 of YakoGL's Mali back end over panthor
# for the NanoPC-T6's Armbian (aarch64-unknown-linux-gnu), into <outdir>.
# README.md beside this script says what the pair is and how dEQP loads it.
#
#   tools/mali/panthor-gles/build.sh [--debug] [--es3] <outdir>
#
#   --debug  the dev profile (default: release)
#   --es3    without `report-es2`: the pair reports OpenGL ES 3.0 (default:
#            ES 2.0, what a dEQP-GLES2 run needs)
#
# The toolchain. The board's ABI floors are GLIBC 2.41 and GLIBCXX 3.4.33
# (Debian trixie, the Armbian the board runs), so the default links against a
# Debian trixie arm64 sysroot with clang and lld: nothing linked can need a
# GLIBC version the board lacks. The sysroot is four pinned Debian packages
# (PKGS below, sha256-checked), unpacked once into
#   $YAKOGL_MALI_SYSROOT, else
#   ${CONNECTED_DEPS_DIR:-${CCFARM_DEPS_DIR:-~/.local/share/connected/deps/yakogl}}/debian-trixie-arm64-sysroot
# (the layout directory of tools/cts/deps-env.sh). The linker is lld:
# `ld.lld` on PATH, else the `rust-lld` of the active Rust toolchain.
#
# Override: YAKOGL_MALI_CC=<compiler driver> (for example
# aarch64-linux-gnu-gcc of a host whose cross glibc is at most 2.41) links
# with that driver and its own sysroot instead; check.sh still checks the
# GLIBC ceiling.
#
# Needs: cargo with the aarch64-unknown-linux-gnu target
# (rustup target add aarch64-unknown-linux-gnu), clang (unless YAKOGL_MALI_CC),
# and for the first sysroot: curl or python3, ar, tar with xz.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
target=aarch64-unknown-linux-gnu
profile=release
features=
out=
while [ $# -gt 0 ]; do
	case "$1" in
	--debug) profile=debug ;;
	--es3) features=--no-default-features ;;
	-h | --help)
		sed -n '5,30p' "$0"
		exit 0
		;;
	-*)
		echo "build.sh: unknown option $1" >&2
		exit 2
		;;
	*)
		[ -z "$out" ] || {
			echo "build.sh: one <outdir> only" >&2
			exit 2
		}
		out=$1
		;;
	esac
	shift
done
[ -n "$out" ] || {
	echo "usage: build.sh [--debug] [--es3] <outdir>" >&2
	exit 2
}

# Debian trixie arm64 (main, 2026-11-07): name version pool path sha256.
PKGS="libc6 2.41-12+deb13u4 pool/main/g/glibc/libc6_2.41-12+deb13u4_arm64.deb 8784eda966b189c777a384dac5ce009e8fc9b52d006926c5a013e7fa8aa688cc
libc6-dev 2.41-12+deb13u4 pool/main/g/glibc/libc6-dev_2.41-12+deb13u4_arm64.deb 71e373cb5e9da975b02fb4af555929cb1977560e2dd8245de4eb8a7b03677a08
libgcc-s1 14.2.0-19 pool/main/g/gcc-14/libgcc-s1_14.2.0-19_arm64.deb 1108bc87879833d6d9a145f22a4a15cddb34e065b4b5f4b97bee586adbac2851
libgcc-14-dev 14.2.0-19 pool/main/g/gcc-14/libgcc-14-dev_14.2.0-19_arm64.deb 64b8ebc7182a69aa9525df6eab5e7eb849b947e350d821549f8a8a3e9b19e5ba"
# A package that leaves the pool at a point release stays on the snapshot.
MIRRORS="https://deb.debian.org/debian https://snapshot.debian.org/archive/debian/20261107T000000Z"

fetch() { # url file
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL --retry 2 -o "$2" "$1"
	else
		python3 -I -c 'import sys, urllib.request as u; open(sys.argv[2], "wb").write(u.urlopen(sys.argv[1], timeout=120).read())' "$1" "$2"
	fi
}

sha256_of() { sha256sum "$1" | cut -d' ' -f1; }

# The stamp names every package, so a change to PKGS makes a new sysroot.
pkgs_stamp() { printf '%s\n' "$PKGS" | sha256sum | cut -c1-16; }

make_sysroot() { # dir
	dir=$1
	stamp=$(pkgs_stamp)
	[ "$(cat "$dir/.yakogl-sysroot" 2>/dev/null)" = "$stamp" ] && return 0
	echo "build.sh: making the Debian trixie arm64 sysroot in $dir" >&2
	part=$dir.part
	rm -rf "$part"
	mkdir -p "$part/.debs"
	printf '%s\n' "$PKGS" | while read -r name ver path sum; do
		deb=$part/.debs/${path##*/}
		got=
		for m in $MIRRORS; do
			if fetch "$m/$path" "$deb" 2>/dev/null && [ "$(sha256_of "$deb")" = "$sum" ]; then
				got=1
				break
			fi
		done
		[ -n "$got" ] || {
			echo "build.sh: could not fetch $name $ver ($path) with sha256 $sum" >&2
			exit 1
		}
		member=$(ar t "$deb" | grep '^data\.tar')
		case "$member" in
		*.xz) z=-J ;;
		*.zst) z=--zstd ;;
		*.gz) z=-z ;;
		*) z= ;;
		esac
		ar p "$deb" "$member" | tar -x $z -f - -C "$part"
	done
	# merged /usr, as base-files makes it: libc.so's linker script names
	# /lib/aarch64-linux-gnu/libc.so.6 and /lib/ld-linux-aarch64.so.1
	[ -e "$part/lib" ] || ln -s usr/lib "$part/lib"
	rm -rf "$part/.debs"
	echo "$stamp" >"$part/.yakogl-sysroot"
	rm -rf "$dir"
	mv "$part" "$dir"
}

work=${CARGO_TARGET_DIR:-$here/target}
mkdir -p "$work" "$out"
out=$(cd "$out" && pwd)

if [ -n "${YAKOGL_MALI_CC:-}" ]; then
	cc_driver=$YAKOGL_MALI_CC
	echo "build.sh: linking with YAKOGL_MALI_CC=$cc_driver" >&2
else
	command -v clang >/dev/null 2>&1 || {
		echo "build.sh: needs clang (or YAKOGL_MALI_CC=<an aarch64 linux gcc>)" >&2
		exit 1
	}
	deps=${CONNECTED_DEPS_DIR:-${CCFARM_DEPS_DIR:-$HOME/.local/share/connected/deps/yakogl}}
	sysroot=${YAKOGL_MALI_SYSROOT:-$deps/debian-trixie-arm64-sysroot}
	if [ -z "${YAKOGL_MALI_SYSROOT:-}" ]; then
		make_sysroot "$sysroot"
	fi
	[ -e "$sysroot/usr/lib/aarch64-linux-gnu/libc.so.6" ] || {
		echo "build.sh: $sysroot is not an arm64 glibc sysroot (no usr/lib/aarch64-linux-gnu/libc.so.6)" >&2
		exit 1
	}
	if command -v ld.lld >/dev/null 2>&1; then
		lld=$(command -v ld.lld)
	else
		lld=$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/gcc-ld/ld.lld
		[ -x "$lld" ] || {
			echo "build.sh: no ld.lld on PATH and no rust-lld wrapper at $lld" >&2
			exit 1
		}
	fi
	# the driver cargo links with: one program, so a script
	cc_driver=$work/aarch64-trixie-cc
	cat >"$cc_driver" <<EOF
#!/bin/sh
# Generated by tools/mali/panthor-gles/build.sh.
exec clang --target=aarch64-linux-gnu --sysroot="$sysroot" -fuse-ld=lld --ld-path="$lld" "\$@"
EOF
	chmod +x "$cc_driver"
fi

# The library: the crate's cdylib, its DT_SONAME the one dEQP opens. -z defs:
# everything resolves against the board's libc, libm, libgcc_s and the dynamic
# loader at link time, so no symbol is left for dlopen to miss. -Bsymbolic:
# the library's own calls of its gl*/egl* bind to itself, whatever else the
# process has loaded under those names (the board's glvnd).
# The variables the panthor crate's build.rs takes a cross compiler from are
# cleared: with none it does not compile gl_test.c, which this library does
# not link.
cargo_profile=
[ "$profile" = release ] && cargo_profile=--release
# shellcheck disable=SC2086 # $features and $cargo_profile are one word or none
env -u CC_aarch64_unknown_linux_gnu -u CC_aarch64-unknown-linux-gnu -u TARGET_CC \
	-u CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
	CARGO_TARGET_DIR="$work" cargo rustc --manifest-path "$here/Cargo.toml" --locked $cargo_profile $features \
	--target "$target" --lib --crate-type cdylib \
	--config "target.$target.linker=\"$cc_driver\"" -- \
	-C link-arg=-Wl,-soname,libGLESv2.so.2 \
	-C link-arg=-Wl,-z,defs \
	-C link-arg=-Wl,-Bsymbolic \
	-C link-arg=-Wl,--build-id
so=$work/$target/$profile/libyakogl_mali_gles.so
[ -f "$so" ] || {
	echo "build.sh: cargo finished but $so is not there" >&2
	exit 1
}
rm -f "$out/libGLESv2.so.2" "$out/libEGL.so.1"
cp "$so" "$out/libGLESv2.so.2"

# libEGL.so.1: the forwarder CMakeLists.txt also makes. The EGL entry points
# live in libGLESv2.so.2 (EGL and GL share one state); this object carries the
# SONAME applications open, and its DT_NEEDED libGLESv2.so.2 (found beside it
# through its DT_RUNPATH $ORIGIN) is what dlsym(libEGL, "egl...") searches.
# The anchor is a reference into libGLESv2 so --as-needed cannot drop it.
# shellcheck disable=SC2016 # $ORIGIN is the loader's, not the shell's
printf '%s\n' \
	'/* Generated by tools/mali/panthor-gles/build.sh: the libEGL.so.1 forwarder. */' \
	'extern unsigned int eglGetError(void);' \
	'__attribute__((visibility("hidden")))' \
	'unsigned int (*const yakogl_egl_anchor)(void) = eglGetError;' |
	"$cc_driver" -x c - -shared -fPIC -O2 -o "$out/libEGL.so.1" \
		-Wl,-soname,libEGL.so.1 -Wl,--no-as-needed -Wl,-z,defs -Wl,--build-id \
		-Wl,-rpath,'$ORIGIN' -L"$out" -l:libGLESv2.so.2

echo "build.sh: $out/libGLESv2.so.2 and $out/libEGL.so.1 ($profile, ${features:-report-es2})"
