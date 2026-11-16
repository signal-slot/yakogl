#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# regen.sh: regenerate stm32mp25-dk-initial-env.txt, ST's default U-Boot
# environment of the STM32MP257F-DK (stm32mp25_defconfig, DEVICE_TREE
# stm32mp257f-dk), with `make u-boot-initial-env` (README.md here).
#
#   regen.sh           write stm32mp25-dk-initial-env.txt (only when it differs)
#   regen.sh --check   build and compare; exit 1 on a difference, write nothing
#
# Environment:
#   ST_UBOOT_TREE  ST's U-Boot checkout (read only: the build is out of tree).
#                  Default: $CONNECTED_DEPS_DIR/st-uboot, cloned at the pinned
#                  tag when absent.
#   ST_ENV_BUILD   build tree. Default: $CONNECTED_DEPS_DIR/st-uboot-build,
#                  else $TMPDIR/st-uboot-build. Never /tmp or /var/tmp.
#   CROSS_COMPILE  an aarch64 prefix. Default: aarch64-linux-gnu-, else the
#                  Zephyr SDK's aarch64-zephyr-elf- (ZEPHYR_SDK_INSTALL_DIR or
#                  $CONNECTED_DEPS_DIR/zephyr-sdk-*).
#
# Exit 0: done. Exit 77: no toolchain/tree (a skip). Other: failure.
set -euo pipefail

ST_TAG=v2023.10-stm32mp-r3.1
ST_COMMIT=f8917a01b48e0f3bc4595cb39e2d5431774004d5 # the commit the tag peels to
ST_GIT=https://github.com/STMicroelectronics/u-boot
DEFCONFIG=stm32mp25_defconfig
DTB=stm32mp257f-dk

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$here/stm32mp25-dk-initial-env.txt
check=0
case "${1:-}" in
--check) check=1 ;;
"") ;;
*) echo "usage: regen.sh [--check]" >&2; exit 2 ;;
esac

say() { echo "regen.sh: $*" >&2; }

deps=${CONNECTED_DEPS_DIR:-}
tree=${ST_UBOOT_TREE:-}
if [ -z "$tree" ] && [ -n "$deps" ]; then tree=$deps/st-uboot; fi
if [ -z "$tree" ]; then say "set ST_UBOOT_TREE or CONNECTED_DEPS_DIR"; exit 77; fi

build=${ST_ENV_BUILD:-}
if [ -z "$build" ] && [ -n "$deps" ]; then build=$deps/st-uboot-build; fi
if [ -z "$build" ] && [ -n "${TMPDIR:-}" ]; then build=$TMPDIR/st-uboot-build; fi
if [ -z "$build" ]; then say "set ST_ENV_BUILD, CONNECTED_DEPS_DIR or TMPDIR"; exit 77; fi
case "$build" in
/tmp | /tmp/* | /var/tmp | /var/tmp/*) say "the build tree must not be in /tmp"; exit 2 ;;
esac

if [ ! -e "$tree/Makefile" ]; then
	say "cloning $ST_TAG into $tree"
	git clone --quiet --depth 1 --branch "$ST_TAG" "$ST_GIT" "$tree" || exit 77
fi
head=$(git -C "$tree" rev-parse HEAD)
if [ "$head" != "$ST_COMMIT" ]; then
	say "$tree is at $head, expected $ST_COMMIT ($ST_TAG)"
	exit 1
fi

cross=${CROSS_COMPILE:-}
if [ -z "$cross" ] && command -v aarch64-linux-gnu-gcc >/dev/null; then cross=aarch64-linux-gnu-; fi
if [ -z "$cross" ]; then
	for sdk in "${ZEPHYR_SDK_INSTALL_DIR:-}" "$deps"/zephyr-sdk-*; do
		p=$sdk/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-
		if [ -n "$sdk" ] && [ -x "${p}gcc" ]; then cross=$p; break; fi
	done
fi
if [ -z "$cross" ]; then say "no aarch64 cross compiler"; exit 77; fi

mkdir -p "$build"
export DEVICE_TREE=$DTB CROSS_COMPILE=$cross
mk() { make -s -C "$tree" O="$build" "$@"; }
mk $DEFCONFIG >&2
mk u-boot-initial-env >&2

# u-boot-initial-env is what the build writes: sorted, one var=value per
# line. Keep it byte for byte.
gen=$build/u-boot-initial-env
if [ ! -s "$gen" ]; then say "no $gen"; exit 1; fi
if [ "$check" = 1 ]; then
	if cmp -s "$gen" "$out"; then say "ok: $out reproduces"; exit 0; fi
	say "differs from $out"
	exit 1
fi
if cmp -s "$gen" "$out" 2>/dev/null; then
	say "unchanged: $out"
else
	cp "$gen" "$out"
	say "wrote $out"
fi
