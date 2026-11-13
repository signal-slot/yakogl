#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build-uboot.sh: build mainline U-Boot v2023.10, qemu_arm64_defconfig plus
# mp2-standalone.config, for run.sh (README.md in this directory). Prints the
# path of u-boot.bin on its last line.
#
# It is ../../uboot-mp2/qemu/build-uboot.sh's approach with its own fragment
# and its own cache directory: that script's fragment overrides bootcmd, this
# one keeps the distro boot. The cross compiler search and the cache rules are
# that script's functions, sourced.
#
# Environment:
#   MP2S_QEMU_CACHE     cache directory (build tree, run files). Default:
#                       $CONNECTED_DEPS_DIR/mp2-standalone-qemu, else
#                       $TMPDIR/mp2-standalone-qemu, else
#                       ~/.cache/yakogl/mp2-standalone-qemu. Never /tmp or
#                       /var/tmp, never inside the repository.
#   UBOOT_V2023_10_SRC  a U-Boot v2023.10 tree (read only: the build is out of
#                       tree). Default: $CONNECTED_DEPS_DIR/u-boot-v2023.10,
#                       else $CACHE/u-boot-v2023.10, cloned from source.denx.de
#                       (into $CONNECTED_DEPS_DIR when set) when neither exists.
#   CROSS_COMPILE       an aarch64 cross prefix; default as in the sourced
#                       script (the Zephyr SDK's under $CONNECTED_DEPS_DIR).
#
# Exit 0: built (or already built from the same fragment and source). Exit 77:
# no cross compiler (run.sh turns that into "skip"). Anything else: a failure.
set -euo pipefail

mp2s_here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=../../uboot-mp2/qemu/build-uboot.sh
. "$mp2s_here/../../uboot-mp2/qemu/build-uboot.sh"
here=$mp2s_here
UBOOT_TAG=v2023.10
UBOOT_GIT=https://source.denx.de/u-boot/u-boot.git

# The cache directory (the sourced script's rules, this tool's name).
mp2s_cache() {
	local c=${MP2S_QEMU_CACHE:-}
	if [ -z "$c" ] && [ -n "${CONNECTED_DEPS_DIR:-}" ]; then c=$CONNECTED_DEPS_DIR/mp2-standalone-qemu; fi
	if [ -z "$c" ] && [ -n "${TMPDIR:-}" ]; then
		case "$TMPDIR" in /tmp | /tmp/* | /var/tmp | /var/tmp/*) ;; *) c=$TMPDIR/mp2-standalone-qemu ;; esac
	fi
	[ -n "$c" ] || c=${XDG_CACHE_HOME:-$HOME/.cache}/yakogl/mp2-standalone-qemu
	mp2_qemu_cache_ok "$c" || return 1
	mkdir -p "$c"
	c=$(cd "$c" && pwd -P)
	mp2_qemu_cache_ok "$c" || return 1
	echo "$c"
}

# Sourced by run.sh for mp2s_cache; built only when run.
if [ "${BASH_SOURCE[0]}" != "$0" ]; then return 0; fi

cache=$(mp2s_cache)
cross=$(mp2_qemu_cross)
if [ -z "$cross" ]; then say "no aarch64 cross compiler (set CROSS_COMPILE)"; exit 77; fi

src=${UBOOT_V2023_10_SRC:-}
if [ -z "$src" ]; then
	if [ -n "${CONNECTED_DEPS_DIR:-}" ] && [ -f "$CONNECTED_DEPS_DIR/u-boot-$UBOOT_TAG/Makefile" ]; then
		src=$CONNECTED_DEPS_DIR/u-boot-$UBOOT_TAG
	elif [ -f "$cache/u-boot-$UBOOT_TAG/Makefile" ]; then
		src=$cache/u-boot-$UBOOT_TAG
	else
		src=${CONNECTED_DEPS_DIR:-$cache}/u-boot-$UBOOT_TAG
		say "cloning $UBOOT_GIT $UBOOT_TAG into $src"
		git -c advice.detachedHead=false clone -q --depth 1 -b "$UBOOT_TAG" "$UBOOT_GIT" "$src"
	fi
fi
v=$(sed -n 's/^VERSION = //p; s/^PATCHLEVEL = //p; s/^EXTRAVERSION = //p' "$src/Makefile" | paste -sd. | sed 's/\.$//')
if [ "$v" != "${UBOOT_TAG#v}" ]; then say "$src is U-Boot '$v', not ${UBOOT_TAG#v}"; exit 1; fi

frag=$here/mp2-standalone.config
key=$(printf '%s\n%s\n%s\n' "$(cat "$frag")" "$(cd "$src" && pwd -P)" "$cross" | sha256sum | cut -c1-16)
out=$cache/build-qemu_arm64-$key
if [ -f "$out/u-boot.bin" ]; then
	say "up to date: $out"
	echo "$out/u-boot.bin"
	exit 0
fi

mkdir -p "$out"
say "building U-Boot $UBOOT_TAG qemu_arm64 in $out (CROSS_COMPILE=$cross)"
# merge_config.sh wants "# CONFIG_X is not set" for an off option.
grep -v '^#' "$frag" | grep -v '^$' |
	sed 's/^\(CONFIG_[A-Z0-9_]*\)=n$/# \1 is not set/' > "$out/mp2-standalone.config"
mk() { make -s -C "$src" O="$out" CROSS_COMPILE="$cross" "$@"; }
mk qemu_arm64_defconfig > /dev/null
"$src/scripts/kconfig/merge_config.sh" -m -O "$out" "$out/.config" "$out/mp2-standalone.config" > /dev/null
mk olddefconfig > /dev/null
# merge_config drops silently what Kconfig refused; check every line held.
while IFS= read -r line; do
	if ! grep -qxF "$line" "$out/.config"; then
		say "the fragment's '$line' did not hold in $out/.config"
		exit 1
	fi
done < "$out/mp2-standalone.config"
if ! mk -j"$(nproc)" u-boot.bin > "$out/build.log" 2>&1; then
	tail -n 30 "$out/build.log" >&2
	say "build failed; full log $out/build.log"
	exit 1
fi
echo "$out/u-boot.bin"
