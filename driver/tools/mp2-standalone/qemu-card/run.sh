#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: boots the 'forever' product card of tools/mp2s_product_card.py in
# U-Boot v2023.10 under QEMU, three times (README.md in this directory).
# Builds the redund U-Boot (../qemu/build-uboot.sh redund: the environment in
# two copies on the card), the product script (../build.sh) and the stand-in
# image (../qemu/image.S), then runs card.py. Reuses ../qemu without changing
# it. Prints "PASS|FAIL case: ..." per assertion, "case NAME: PASS|FAIL" per
# case and "card.py: N ok, M failed" last; exit 0 only when nothing failed.
#
# Optional, no board: prints "SKIP: <what>" and exits 0 when an input is
# missing:
#   QEMU_SYSTEM_AARCH64  the QEMU binary; unset: the Zephyr SDK host tools'
#                        under $CONNECTED_DEPS_DIR/zephyr-hosttools when there
#   UBOOT_V2023_10_SRC   a U-Boot v2023.10 tree; unset:
#                        $CONNECTED_DEPS_DIR/u-boot-v2023.10 when it exists
#   CROSS_COMPILE        an aarch64 prefix (default: ../qemu/build-uboot.sh's search)
# Also: MP2S_QEMU_CACHE (the cache, as ../qemu), MP2S_QEMU_KEEP=1 (keep the
# run directory; a failing run keeps it anyway), MP2S_CARD_BOOTS (default 3).
set -euo pipefail

cdir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$cdir/../../../.." && pwd)
skip() { echo "SKIP: $*"; exit 0; }

qemu=${QEMU_SYSTEM_AARCH64:-}
if [ -z "$qemu" ] && [ -n "${CONNECTED_DEPS_DIR:-}" ]; then
	for q in "$CONNECTED_DEPS_DIR"/zephyr-hosttools/sysroots/*/usr/bin/qemu-system-aarch64; do
		if [ -x "$q" ]; then qemu=$q; fi
	done
fi
[ -n "$qemu" ] && [ -x "$qemu" ] || skip "QEMU_SYSTEM_AARCH64 (unset or not executable, and no \$CONNECTED_DEPS_DIR/zephyr-hosttools)"
if [ -z "${UBOOT_V2023_10_SRC:-}" ]; then
	if [ -n "${CONNECTED_DEPS_DIR:-}" ] && [ -f "$CONNECTED_DEPS_DIR/u-boot-v2023.10/Makefile" ]; then
		export UBOOT_V2023_10_SRC=$CONNECTED_DEPS_DIR/u-boot-v2023.10
	else
		skip "UBOOT_V2023_10_SRC (unset, and no \$CONNECTED_DEPS_DIR/u-boot-v2023.10)"
	fi
fi
[ -f "$UBOOT_V2023_10_SRC/Makefile" ] || skip "UBOOT_V2023_10_SRC ($UBOOT_V2023_10_SRC has no Makefile)"
# shellcheck source=../qemu/build-uboot.sh
. "$cdir/../qemu/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
for t in python3 mke2fs sfdisk truncate; do
	command -v "$t" > /dev/null || [ -x "/usr/sbin/$t" ] || [ -x "/sbin/$t" ] || skip "$t (not found)"
done
cache=$(mp2s_cache)

rc=0
uboot=$(bash "$cdir/../qemu/build-uboot.sh" redund) || rc=$?
[ "$rc" -ne 77 ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
[ "$rc" -eq 0 ] || { echo "run.sh: ../qemu/build-uboot.sh redund failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
echo "run.sh: U-Boot (redund) $uboot"
echo "run.sh: QEMU $("$qemu" --version | head -n 1)"

run=$(mktemp -d "$cache/card.XXXXXX")
cleanup() { if [ "${MP2S_QEMU_KEEP:-}" != 1 ] && [ "$rc" -eq 0 ]; then rm -rf "$run"; fi; }
trap cleanup EXIT
mkdir -p "$run/logs" "$run/script"

# The product script, the way the runner builds it.
bash "$cdir/../build.sh" "$run/script" | tail -n 1
# The stand-in default.bin (../qemu/image.S): 4 KiB file, image_size 0x2000.
"${cross}gcc" -nostdlib -c -DIMGSZ=0x2000 -o "$run/standin.o" "$cdir/../qemu/image.S"
"${cross}objcopy" -O binary -j .text "$run/standin.o" "$run/standin.bin"
truncate -s 4096 "$run/standin.bin"

python3 -I "$cdir/card.py" --qemu "$qemu" --uboot "$uboot" --image "$run/standin.bin" \
	--script "$run/script/boot.scr.uimg" \
	--fixture "$cdir/../st-env/stm32mp25-dk-initial-env.txt" \
	--card-tool "$root/tools/mp2s_product_card.py" \
	--uboot-script "$root/tools/uboot_script.py" \
	--work "$run/work" --logs "$run/logs" --boots "${MP2S_CARD_BOOTS:-3}" || rc=$?
if [ "$rc" -ne 0 ]; then echo "run.sh: run directory and console logs kept in $run"; fi
exit "$rc"
