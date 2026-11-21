#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: the card-in-a-PC undo (tools/mp2s_card_undo.sh) proved in U-Boot
# v2023.10 under QEMU: count 2, boot 1 ("boots left 1"), card undo on the disk
# image, boot 2 = stock path, no mp2s line (README.md in this directory).
# Reuses ../qemu and ../qemu-product without changing them. Last line:
# "qemu-undo: ok", "qemu-undo: FAIL ..." (exit 1) or "qemu-undo: SKIP <why>"
# (exit 0, an input is missing). Inputs as ../qemu-product/run.sh:
#   QEMU_SYSTEM_AARCH64, UBOOT_V2023_10_SRC (default
#   $CONNECTED_DEPS_DIR/u-boot-v2023.10), CROSS_COMPILE, MP2S_QEMU_CACHE,
#   MP2S_QEMU_KEEP=1 (keep the run directory; a failing run keeps it anyway).
set -euo pipefail

pdir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
skip() { echo "qemu-undo: SKIP $*"; exit 0; }

qemu=${QEMU_SYSTEM_AARCH64:-}
[ -n "$qemu" ] && [ -x "$qemu" ] || skip "QEMU_SYSTEM_AARCH64 (unset or not executable)"
if [ -z "${UBOOT_V2023_10_SRC:-}" ]; then
	if [ -n "${CONNECTED_DEPS_DIR:-}" ] && [ -f "$CONNECTED_DEPS_DIR/u-boot-v2023.10/Makefile" ]; then
		export UBOOT_V2023_10_SRC=$CONNECTED_DEPS_DIR/u-boot-v2023.10
	else
		skip "UBOOT_V2023_10_SRC (unset, and no \$CONNECTED_DEPS_DIR/u-boot-v2023.10)"
	fi
fi
[ -f "$UBOOT_V2023_10_SRC/Makefile" ] || skip "UBOOT_V2023_10_SRC ($UBOOT_V2023_10_SRC has no Makefile)"
export PATH=$PATH:/usr/sbin:/sbin
# shellcheck source=../qemu/build-uboot.sh
. "$pdir/../qemu/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
for t in python3 mke2fs sfdisk truncate debugfs dumpe2fs blkid sha256sum dd; do
	command -v "$t" > /dev/null || skip "$t (not found)"
done
cache=$(mp2s_cache)

rc=0
uboot=$(bash "$pdir/../qemu/build-uboot.sh" redund) || rc=$?
[ "$rc" -ne 77 ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
[ "$rc" -eq 0 ] || { echo "qemu-undo: FAIL ../qemu/build-uboot.sh redund failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
echo "qemu-undo: U-Boot (redund) $uboot"
echo "qemu-undo: QEMU $("$qemu" --version | head -n 1)"

run=$(mktemp -d "$cache/undo.XXXXXX")
cleanup() { if [ "${MP2S_QEMU_KEEP:-}" != 1 ] && [ "$rc" -eq 0 ]; then rm -rf "$run"; fi; }
trap cleanup EXIT
mkdir -p "$run/logs" "$run/script"

bash "$pdir/../build.sh" "$run/script" | tail -n 1
"${cross}gcc" -nostdlib -c -DIMGSZ=0x2000 -o "$run/standin.o" "$pdir/../qemu/image.S"
"${cross}objcopy" -O binary -j .text "$run/standin.o" "$run/standin.bin"
truncate -s 4096 "$run/standin.bin"

python3 -I "$pdir/undo.py" --qemu "$qemu" --uboot "$uboot" --image "$run/standin.bin" \
	--script "$run/script/boot.scr.uimg" \
	--fixture "$pdir/../st-env/stm32mp25-dk-initial-env.txt" \
	--undo "$pdir/../../../../tools/mp2s_card_undo.sh" \
	--work "$run/work" --logs "$run/logs" \
	--uboot-script "$pdir/../../../../tools/uboot_script.py" || rc=$?
if [ "$rc" -ne 0 ]; then echo "qemu-undo: run directory and console logs kept in $run"; fi
exit "$rc"
