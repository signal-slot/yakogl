#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: the DK product script (../mp2-standalone.cmd, built by ../build.sh)
# in U-Boot v2023.10 under QEMU, through ST's distro loop (README.md in this
# directory). Reuses ../qemu (build-uboot.sh, image.S, mkdisk.sh, drive.py)
# without changing it. Prints one "case NAME: PASS|FAIL" line per case and
# "product.py: N ok, M failed" last; exit 0 only when every case passed.
#
# Optional, no board: prints "SKIP: <var>" and exits 0 when an input is
# missing:
#   QEMU_SYSTEM_AARCH64  the QEMU binary (required; no PATH search)
#   UBOOT_V2023_10_SRC   a U-Boot v2023.10 tree; default
#                        $CONNECTED_DEPS_DIR/u-boot-v2023.10 when it exists
#   CROSS_COMPILE        an aarch64 prefix (default: ../qemu/build-uboot.sh's search)
# Also: MP2S_QEMU_CACHE (the cache, as ../qemu), MP2S_QEMU_KEEP=1 (keep the
# run directory; a failing run keeps it anyway), MP2S_PRODUCT_ONLY=NAME (one case).
set -euo pipefail

pdir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
skip() { echo "SKIP: $*"; exit 0; }

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
# shellcheck source=../qemu/build-uboot.sh
. "$pdir/../qemu/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
for t in python3 mke2fs sfdisk truncate; do
	command -v "$t" > /dev/null || [ -x "/usr/sbin/$t" ] || [ -x "/sbin/$t" ] || skip "$t (not found)"
done
cache=$(mp2s_cache)

rc=0
uboot=$(bash "$pdir/../qemu/build-uboot.sh") || rc=$?
[ "$rc" -ne 77 ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
[ "$rc" -eq 0 ] || { echo "run.sh: ../qemu/build-uboot.sh failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
echo "run.sh: U-Boot $uboot"
echo "run.sh: QEMU $("$qemu" --version | head -n 1)"

run=$(mktemp -d "$cache/product.XXXXXX")
cleanup() { if [ "${MP2S_QEMU_KEEP:-}" != 1 ] && [ "$rc" -eq 0 ]; then rm -rf "$run"; fi; }
trap cleanup EXIT
mkdir -p "$run/logs" "$run/script"

# The product script, the way the runner builds it.
bash "$pdir/../build.sh" "$run/script" | tail -n 1
# The stand-in image (../qemu/image.S): 4 KiB file, image_size 0x2000.
"${cross}gcc" -nostdlib -c -DIMGSZ=0x2000 -o "$run/standin.o" "$pdir/../qemu/image.S"
"${cross}objcopy" -O binary -j .text "$run/standin.o" "$run/standin.bin"
truncate -s 4096 "$run/standin.bin"

only=()
if [ -n "${MP2S_PRODUCT_ONLY:-}" ]; then only=(--only "$MP2S_PRODUCT_ONLY"); fi
python3 -I "$pdir/product.py" --qemu "$qemu" --uboot "$uboot" --image "$run/standin.bin" \
	--script "$run/script/boot.scr.uimg" \
	--fixture "$pdir/../st-env/stm32mp25-dk-initial-env.txt" \
	--work "$run/work" --logs "$run/logs" \
	--uboot-script "$pdir/../../../../tools/uboot_script.py" ${only[@]+"${only[@]}"} || rc=$?
if [ "${MP2S_QEMU_KEEP:-}" = 1 ]; then echo "run.sh: logs in $run/logs"; fi
if [ "$rc" -ne 0 ]; then echo "run.sh: run directory and console logs kept in $run"; fi
exit "$rc"
