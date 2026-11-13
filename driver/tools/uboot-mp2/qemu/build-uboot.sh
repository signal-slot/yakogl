#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build-uboot.sh: build mainline U-Boot v2023.10, qemu_arm64_defconfig plus
# the fragment below, for run.sh (README.md in this directory). Prints the
# path of u-boot.bin on its last line.
#
# Environment:
#   MP2_QEMU_CACHE     cache directory (sources, build tree, run files).
#                      Default: $CONNECTED_DEPS_DIR/uboot-mp2-qemu, else
#                      $TMPDIR/uboot-mp2-qemu, else ~/.cache/yakogl/uboot-mp2-qemu.
#                      Never /tmp or /var/tmp, never inside the repository.
#   MP2_QEMU_UBOOT_SRC a U-Boot v2023.10 tree to build from (read only: the
#                      build is out of tree). Default: $CACHE/u-boot-v2023.10,
#                      or $CONNECTED_DEPS_DIR/u-boot-v2023.10 when it exists,
#                      cloned from source.denx.de when neither does.
#   CROSS_COMPILE      an aarch64 cross prefix. Default: aarch64-linux-gnu-,
#                      aarch64-none-elf-, or the Zephyr SDK's aarch64-zephyr-elf-
#                      (ZEPHYR_SDK_INSTALL_DIR or $CONNECTED_DEPS_DIR/zephyr-sdk-*).
#
# Exit 0: built (or already built from the same fragment and source). Exit 77:
# no cross compiler (run.sh turns that into "skip"). Anything else: a failure.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../../.." && pwd)
UBOOT_TAG=v2023.10
UBOOT_GIT=https://source.denx.de/u-boot/u-boot.git

say() { echo "build-uboot.sh: $*" >&2; }

# The cache directory: never a memory-backed temp, never the repository.
mp2_qemu_cache() {
	local c=${MP2_QEMU_CACHE:-}
	if [ -z "$c" ] && [ -n "${CONNECTED_DEPS_DIR:-}" ]; then c=$CONNECTED_DEPS_DIR/uboot-mp2-qemu; fi
	if [ -z "$c" ] && [ -n "${TMPDIR:-}" ]; then
		case "$TMPDIR" in /tmp | /tmp/* | /var/tmp | /var/tmp/*) ;; *) c=$TMPDIR/uboot-mp2-qemu ;; esac
	fi
	[ -n "$c" ] || c=${XDG_CACHE_HOME:-$HOME/.cache}/yakogl/uboot-mp2-qemu
	mp2_qemu_cache_ok "$c" || return 1
	mkdir -p "$c"
	c=$(cd "$c" && pwd -P)
	mp2_qemu_cache_ok "$c" || return 1
	echo "$c"
}
mp2_qemu_cache_ok() {
	case "$1/" in
	/tmp/* | /var/tmp/*) say "cache $1 is under /tmp or /var/tmp; set MP2_QEMU_CACHE"; return 1 ;;
	"$root"/* | "$(cd "$root" && pwd -P)"/*) say "cache $1 is inside the repository; set MP2_QEMU_CACHE"; return 1 ;;
	esac
}

# The aarch64 cross prefix, or nothing.
mp2_qemu_cross() {
	local p d
	if [ -n "${CROSS_COMPILE:-}" ]; then
		command -v "${CROSS_COMPILE}gcc" > /dev/null && echo "$CROSS_COMPILE"
		return 0
	fi
	for p in aarch64-linux-gnu- aarch64-none-elf- aarch64-zephyr-elf-; do
		if command -v "${p}gcc" > /dev/null; then echo "$p"; return 0; fi
	done
	for d in ${ZEPHYR_SDK_INSTALL_DIR:+"$ZEPHYR_SDK_INSTALL_DIR"} ${CONNECTED_DEPS_DIR:+"$CONNECTED_DEPS_DIR"/zephyr-sdk-*}; do
		for p in "$d"/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf- "$d"/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-; do
			if [ -x "${p}gcc" ]; then echo "$p"; return 0; fi
		done
	done
}

# Sourced by run.sh for the two helpers above; built only when run.
if [ "${BASH_SOURCE[0]}" != "$0" ]; then return 0; fi

cache=$(mp2_qemu_cache)
cross=$(mp2_qemu_cross)
if [ -z "$cross" ]; then say "no aarch64 cross compiler (set CROSS_COMPILE)"; exit 77; fi

src=${MP2_QEMU_UBOOT_SRC:-}
if [ -z "$src" ]; then
	if [ -f "$cache/u-boot-$UBOOT_TAG/Makefile" ]; then src=$cache/u-boot-$UBOOT_TAG
	elif [ -n "${CONNECTED_DEPS_DIR:-}" ] && [ -f "$CONNECTED_DEPS_DIR/u-boot-$UBOOT_TAG/Makefile" ]; then
		src=$CONNECTED_DEPS_DIR/u-boot-$UBOOT_TAG
	else
		src=$cache/u-boot-$UBOOT_TAG
		say "cloning $UBOOT_GIT $UBOOT_TAG into $src"
		git -c advice.detachedHead=false clone -q --depth 1 -b "$UBOOT_TAG" "$UBOOT_GIT" "$src"
	fi
fi
v=$(sed -n 's/^VERSION = //p; s/^PATCHLEVEL = //p; s/^EXTRAVERSION = //p' "$src/Makefile" | paste -sd. | sed 's/\.$//')
if [ "$v" != "${UBOOT_TAG#v}" ]; then say "$src is U-Boot '$v', not ${UBOOT_TAG#v}"; exit 1; fi

# The fragment: what the scripts need that qemu_arm64_defconfig lacks, and a
# bootcmd that only prints, so the typed line's "run bootcmd" shows the
# fallback without booting anything. Nothing here touches the environment's
# storage (it stays CFI flash at 0x04000000, QEMU's second pflash).
frag=$(cat << 'EOF'
CONFIG_MMC=y
CONFIG_DM_MMC=y
CONFIG_MMC_SDHCI=y
CONFIG_MMC_PCI=y
CONFIG_CMD_MMC=y
CONFIG_CMD_EXT4=y
CONFIG_CMD_FS_GENERIC=y
CONFIG_CMD_ITEST=y
CONFIG_CMD_SETEXPR=y
CONFIG_CMD_SOURCE=y
CONFIG_CMD_CACHE=y
CONFIG_CMD_DHCP=y
CONFIG_CMD_TFTPBOOT=y
CONFIG_CMD_GO=y
CONFIG_CMD_SAVEENV=y
CONFIG_TOOLS_MKEFICAPSULE=n
CONFIG_USE_BOOTCOMMAND=y
CONFIG_BOOTCOMMAND="echo mp2q: stock bootcmd ran"
CONFIG_BOOTDELAY=2
EOF
)
key=$(printf '%s\n%s\n%s\n' "$frag" "$(cd "$src" && pwd -P)" "$cross" | sha256sum | cut -c1-16)
out=$cache/build-qemu_arm64-$key
if [ -f "$out/u-boot.bin" ]; then
	say "up to date: $out"
	echo "$out/u-boot.bin"
	exit 0
fi

mkdir -p "$out"
say "building U-Boot $UBOOT_TAG qemu_arm64 in $out (CROSS_COMPILE=$cross)"
printf '%s\n' "$frag" | sed 's/^\(CONFIG_[A-Z0-9_]*\)=n$/# \1 is not set/' > "$out/mp2-qemu.config"
mk() { make -s -C "$src" O="$out" CROSS_COMPILE="$cross" "$@"; }
mk qemu_arm64_defconfig > /dev/null
"$src/scripts/kconfig/merge_config.sh" -m -O "$out" "$out/.config" "$out/mp2-qemu.config" > /dev/null
mk olddefconfig > /dev/null
# merge_config drops silently what Kconfig refused; check every line held.
while IFS= read -r line; do
	if ! grep -qxF "$line" "$out/.config"; then
		say "the fragment's '$line' did not hold in $out/.config"
		exit 1
	fi
done < "$out/mp2-qemu.config"
if ! mk -j"$(nproc)" u-boot.bin > "$out/build.log" 2>&1; then
	tail -n 30 "$out/build.log" >&2
	say "build failed; full log $out/build.log"
	exit 1
fi
echo "$out/u-boot.bin"
