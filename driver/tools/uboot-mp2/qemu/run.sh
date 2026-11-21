#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: run the DK one-shot scripts (../mp2-mmc.cmd, ../mp2-net.cmd) in
# mainline U-Boot v2023.10 on QEMU's virt machine (README.md in this
# directory). Optional: prints "run.sh: skip: <why>" and exits 0 when
# qemu-system-aarch64 (mp2_qemu_bin below), an aarch64 cross compiler or a host
# tool is missing. The summary line is "run.sh: N ok, M failed".
# `run.sh --which-qemu` prints the QEMU a run would use and exits 0, or exits 1
# printing nothing (tools/setup-gate-host.sh --check asks it).
#
# Environment: MP2_QEMU (the QEMU binary), MP2_QEMU_MEM (default 2G: virt RAM
# starts at 0x40000000, so 2 GiB holds the DK window 0xa0000000..0xb0000000),
# MP2_QEMU_BIG_TFTP=0 (drops the 256 MiB + 1 TFTP case), MP2_QEMU_KEEP=1
# (keeps the run directory), and build-uboot.sh's MP2_QEMU_CACHE,
# MP2_QEMU_UBOOT_SRC, CROSS_COMPILE.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
skip() { echo "run.sh: skip: $*"; exit 0; }

# The QEMU: MP2_QEMU when it is set and not empty (the only one then: a name that does not run
# is a skip, never another QEMU), else qemu-system-aarch64 on PATH, else the
# Zephyr SDK host tools' (sysroots/x86_64-pokysdk-linux/usr/bin/) in the places
# build-uboot.sh's mp2_qemu_cross looks for the SDK: ZEPHYR_SDK_INSTALL_DIR,
# $CONNECTED_DEPS_DIR/zephyr-sdk-*, and the dependency layout's zephyr-sdk-*
# (tools/cts/deps-env.sh --layout; its MP2_QEMU entry installs them at
# zephyr-sdk-hosttools-0.10, and deps-env.sh prints MP2_QEMU for it).
mp2_qemu_bin() {
	local d q layout
	if [ -n "${MP2_QEMU:-}" ]; then
		if [ -x "$MP2_QEMU" ]; then echo "$MP2_QEMU"; fi
		return 0
	fi
	if command -v qemu-system-aarch64 > /dev/null; then command -v qemu-system-aarch64; return 0; fi
	layout=$("$here/../../../../tools/cts/deps-env.sh" --layout 2> /dev/null || true)
	for d in ${ZEPHYR_SDK_INSTALL_DIR:+"$ZEPHYR_SDK_INSTALL_DIR"} \
		${CONNECTED_DEPS_DIR:+"$CONNECTED_DEPS_DIR"/zephyr-sdk-*} ${layout:+"$layout"/zephyr-sdk-*}; do
		q=$d/sysroots/x86_64-pokysdk-linux/usr/bin/qemu-system-aarch64
		if [ -x "$q" ]; then echo "$q"; return 0; fi
	done
}
qemu=$(mp2_qemu_bin)
if [ "${1-}" = --which-qemu ]; then
	[ -n "$qemu" ] || exit 1
	echo "$qemu"
	exit 0
fi
[ -n "$qemu" ] || skip "no qemu-system-aarch64: MP2_QEMU='${MP2_QEMU-}' (unset: PATH, then the Zephyr SDK host tools under ZEPHYR_SDK_INSTALL_DIR, \$CONNECTED_DEPS_DIR/zephyr-sdk-* or the layout's zephyr-sdk-*; tools/cts/deps-env.sh --build MP2_QEMU)"
# shellcheck source=build-uboot.sh
. "$here/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "no aarch64 cross compiler (set CROSS_COMPILE)"
for t in python3 mke2fs sfdisk truncate sha256sum; do
	command -v "$t" > /dev/null || [ -x "/usr/sbin/$t" ] || [ -x "/sbin/$t" ] || skip "$t not found"
done
export PATH=$PATH:/usr/sbin:/sbin
cache=$(mp2_qemu_cache)

rc=0
uboot=$(bash "$here/build-uboot.sh") || rc=$?
[ "$rc" -ne 77 ] || skip "no aarch64 cross compiler (set CROSS_COMPILE)"
[ "$rc" -eq 0 ] || { echo "run.sh: build-uboot.sh failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
echo "run.sh: U-Boot $uboot"
echo "run.sh: QEMU $qemu: $("$qemu" --version | head -n 1)"

run=$(mktemp -d "$cache/run.XXXXXX")
cleanup() { if [ "${MP2_QEMU_KEEP:-}" != 1 ] && [ "$rc" -eq 0 ]; then rm -rf "$run"; fi; }
trap cleanup EXIT
stage=$run/stage/yakogl tftp=$run/tftp
mkdir -p "$stage" "$tftp" "$run/logs" "$run/img"

# The scripts, with the mkimage of the same U-Boot build (build.sh takes
# mkimage from PATH).
SOURCE_DATE_EPOCH=0 PATH="$(dirname "$uboot")/tools:$PATH" bash "$here/../build.sh" "$run/scr" > /dev/null

# The images: an arm64 Image header and code that returns (image.S).
img() { # NAME IMGSZ
	"${cross}gcc" -nostdlib -c -DIMGSZ="$2" -o "$run/img/$1.o" "$here/image.S"
	"${cross}objcopy" -O binary -j .text "$run/img/$1.o" "$run/img/$1.bin"
	truncate -s 4096 "$run/img/$1.bin"
}
img ok 0x2000
img bigimg 0x10000001
img long 0xff00000
cp "$run/img/ok.bin" "$run/img/big.bin"
truncate -s $((0x10000001)) "$run/img/big.bin"   # sparse: 256 MiB + 1
truncate -s 4096 "$run/img/nomagic.bin"
# sparse, made here at run time (never in the tree): 255 MiB, the most the
# 256 MiB window takes; the power-cut cases load it
truncate -s $((255 * 1024 * 1024)) "$run/img/long.bin"

cp "$run/scr/mp2-mmc.scr" "$stage/mp2-mmc.scr"
cp "$run/img/ok.bin" "$stage/mp2z.bin"
cp "$run/img/bigimg.bin" "$run/img/nomagic.bin" "$stage/"
cp --sparse=always "$run/img/big.bin" "$stage/big.bin"
# On the card long.bin is 32 MiB of real data, not holes (mke2fs -d skips zero
# blocks too), with the header's image_size still 255 MiB: ext4load then reads
# the card, which drive.py throttles, so the load window has a floor.
{ head -c 4096 "$run/img/long.bin"
  head -c $((32 * 1024 * 1024 - 4096)) /dev/zero | tr '\0' 'x'; } > "$stage/long.bin"
cp "$run/scr/mp2-net.scr" "$tftp/mp2-net.scr"
cp "$run/img/ok.bin" "$tftp/mp2z.bin"
cp "$run/img/bigimg.bin" "$tftp/bigimg.bin"
cp --sparse=always "$run/img/long.bin" "$tftp/long.bin"
if [ "${MP2_QEMU_BIG_TFTP:-1}" = 1 ]; then cp --sparse=always "$run/img/big.bin" "$tftp/big.bin"; fi

# The SD card: GPT, partition 1 a placeholder, partition 2 the ext4 "bootfs"
# with /yakogl/ (the DK stages under /boot/yakogl/, i.e. /yakogl/ inside it).
disk=$run/sd.img
truncate -s 64M "$disk"   # sparse; an SD card size is a power of 2
printf 'label: gpt\nstart=2048, size=2048, name=fsbl\nstart=4096, size=126000, name=bootfs\n' |
	sfdisk -q "$disk"
mke2fs -q -F -t ext4 -L bootfs -d "$run/stage" -E offset=$((4096 * 512)) "$disk" 63000k

# The environment's flash (QEMU virt's second pflash, U-Boot's ENV_ADDR).
truncate -s 64M "$run/env.img"

big=()
if [ "${MP2_QEMU_BIG_TFTP:-1}" = 1 ]; then big=(--big-tftp); fi
python3 -I "$here/drive.py" --qemu "$qemu" --uboot "$uboot" --disk "$disk" \
	--env "$run/env.img" --tftp "$tftp" --mem "${MP2_QEMU_MEM:-2G}" \
	--logs "$run/logs" "${big[@]}" || rc=$?
if [ "$rc" -ne 0 ]; then echo "run.sh: console logs kept in $run/logs"; fi
exit "$rc"
