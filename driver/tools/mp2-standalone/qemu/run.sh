#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: the QEMU harness of the DK standalone scripts (README.md in this
# directory). Builds what it needs (U-Boot v2023.10, the stand-in image), then
# runs the cases with drive.py: N consecutive QEMU boots per case on one
# environment flash and one GPT disk with a bootfs partition. Prints
# "run.sh: N ok, M failed", or "run.sh: skip: <why>" and exits 0 when a tool
# is missing (never red for a missing tool).
#
# Environment: MP2_QEMU or QEMU_SYSTEM_AARCH64 (the QEMU binary, else
# qemu-system-aarch64 on PATH), UBOOT_V2023_10_SRC, CROSS_COMPILE,
# MP2S_QEMU_CACHE (see build-uboot.sh), MP2S_QEMU_KEEP=1 (keeps the run
# directory; a failing run keeps it anyway), MP2S_QEMU_ONLY=NAME (one case).
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
skip() { echo "run.sh: skip: $*"; exit 0; }

qemu=${MP2_QEMU:-${QEMU_SYSTEM_AARCH64:-$(command -v qemu-system-aarch64 || true)}}
# Last resort: the Zephyr SDK host tools installed into the deps cache.
if [ -z "$qemu" ] && [ -n "${CONNECTED_DEPS_DIR:-}" ]; then
	for q in "$CONNECTED_DEPS_DIR"/zephyr-hosttools/sysroots/*/usr/bin/qemu-system-aarch64; do
		if [ -x "$q" ]; then qemu=$q; fi
	done
fi
[ -n "$qemu" ] && [ -x "$qemu" ] || skip "qemu-system-aarch64 not on PATH (or set MP2_QEMU / QEMU_SYSTEM_AARCH64)"
# shellcheck source=build-uboot.sh
. "$here/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "no aarch64 cross compiler (set CROSS_COMPILE)"
for t in python3 mke2fs sfdisk truncate; do
	command -v "$t" > /dev/null || [ -x "/usr/sbin/$t" ] || [ -x "/sbin/$t" ] || skip "$t not found"
done
cache=$(mp2s_cache)

rc=0
uboot=$(bash "$here/build-uboot.sh") || rc=$?
[ "$rc" -ne 77 ] || skip "no aarch64 cross compiler (set CROSS_COMPILE)"
[ "$rc" -eq 0 ] || { echo "run.sh: build-uboot.sh failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
echo "run.sh: U-Boot $uboot"
echo "run.sh: QEMU $("$qemu" --version | head -n 1)"

run=$(mktemp -d "$cache/run.XXXXXX")
cleanup() { if [ "${MP2S_QEMU_KEEP:-}" != 1 ] && [ "$rc" -eq 0 ]; then rm -rf "$run"; fi; }
trap cleanup EXIT
mkdir -p "$run/logs"

# The stand-in image: an arm64 Image header and code that returns (image.S).
"${cross}gcc" -nostdlib -c -DIMGSZ=0x2000 -o "$run/standin.o" "$here/image.S"
"${cross}objcopy" -O binary -j .text "$run/standin.o" "$run/standin.bin"
truncate -s 4096 "$run/standin.bin"

only=()
if [ -n "${MP2S_QEMU_ONLY:-}" ]; then only=(--only "$MP2S_QEMU_ONLY"); fi
python3 -I "$here/drive.py" --qemu "$qemu" --uboot "$uboot" --image "$run/standin.bin" \
	--work "$run/work" --uboot-script "$here/../../../../tools/uboot_script.py" \
	--logs "$run/logs" ${only[@]+"${only[@]}"} || rc=$?
if [ "$rc" -ne 0 ]; then echo "run.sh: run directory and console logs kept in $run"; fi
exit "$rc"
