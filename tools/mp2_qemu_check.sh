#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2_qemu_check.sh -- report the inputs of the full gate's uboot-mp2 qemu row
# (tools/check.sh mp2_qemu_row, driver/tools/uboot-mp2/qemu/run.sh).
#
#   tools/mp2_qemu_check.sh
#     Print one "ok <name> <path>" or "missing <name> <hint>" line per input,
#     in the words of tools/setup-gate-host.sh --check. Exit 0 when every
#     input is present, 1 otherwise. Nothing is built, fetched or created.
#
# The inputs are the ones run.sh and build-uboot.sh look for:
#   qemu-system-aarch64  MP2_QEMU, else qemu-system-aarch64 on PATH (run.sh
#                        skips without it: the row is a gap)
#   aarch64-cross        build-uboot.sh's mp2_qemu_cross: CROSS_COMPILE, else
#                        aarch64-linux-gnu-, aarch64-none-elf-,
#                        aarch64-zephyr-elf- on PATH, else the Zephyr SDK under
#                        ZEPHYR_SDK_INSTALL_DIR or $CONNECTED_DEPS_DIR/zephyr-sdk-*
#                        (run.sh skips without it); run.sh also runs the
#                        prefix's objcopy
#   python3 mke2fs sfdisk truncate sha256sum dd
#                        run.sh's host tools, on PATH or in /usr/sbin, /sbin
#                        (run.sh skips without one)
#   git                  run.sh's fetch_check runs on every run and
#                        build-uboot.sh fetches U-Boot with it (without it the
#                        row fails, it is not skipped)
#   make                 build-uboot.sh builds U-Boot with it on a host whose
#                        cache holds no build yet (the row fails without it)
#
# Environment: MP2_QEMU, CROSS_COMPILE, ZEPHYR_SDK_INSTALL_DIR,
# CONNECTED_DEPS_DIR as run.sh reads them. MP2_QEMU_CHECK_SBIN (default
# "/usr/sbin /sbin") names the directories searched besides PATH; the
# self-test (tools/mp2_qemu_check_selftest.sh) sets it.
set -uo pipefail

case "${BASH_SOURCE[0]}" in */*) here=${BASH_SOURCE[0]%/*} ;; *) here=. ;; esac
here=$(cd "$here" && pwd) || exit 2
qemu_dir=$here/../driver/tools/uboot-mp2/qemu

case "${1-}" in
"") ;;
-h | --help)
  sed -n '2,/^set /{/^set /d;s/^# \{0,1\}//;p}' "$0"
  exit 0
  ;;
*)
  echo "usage: $0 (no arguments)" >&2
  exit 2
  ;;
esac

missing_count=0

# The same format as tools/setup-gate-host.sh --check.
line() {
  local status=$1 name=$2
  shift 2
  if [ "$status" = missing ]; then
    missing_count=$((missing_count + 1))
  fi
  printf '%-7s %-38s %s\n' "$status" "$name" "$*"
}

# qemu-system-aarch64: run.sh's own test.
if [ -n "${MP2_QEMU:-}" ]; then
  if [ -x "$MP2_QEMU" ]; then
    line ok qemu-system-aarch64 "$MP2_QEMU (MP2_QEMU)"
  else
    line missing qemu-system-aarch64 "MP2_QEMU=$MP2_QEMU is not executable (the uboot-mp2 qemu row would gap): point it at a qemu-system-aarch64 binary"
  fi
elif q=$(command -v qemu-system-aarch64) && [ -x "$q" ]; then
  line ok qemu-system-aarch64 "$q"
else
  line missing qemu-system-aarch64 "not on PATH and MP2_QEMU unset (the uboot-mp2 qemu row would gap): distro package qemu-system-arm (Debian) / app-emulation/qemu with aarch64-softmmu (Gentoo), or set MP2_QEMU"
fi

# The aarch64 cross prefix: build-uboot.sh's mp2_qemu_cross itself, so this
# list cannot drift from the row's.
cross=$(
  # shellcheck source=../driver/tools/uboot-mp2/qemu/build-uboot.sh
  . "$qemu_dir/build-uboot.sh" && mp2_qemu_cross
)
if [ -z "$cross" ]; then
  if [ -n "${CROSS_COMPILE:-}" ]; then
    line missing aarch64-cross "CROSS_COMPILE=$CROSS_COMPILE but ${CROSS_COMPILE}gcc is not runnable (the uboot-mp2 qemu row would gap)"
  else
    line missing aarch64-cross "no aarch64-linux-gnu-gcc, aarch64-none-elf-gcc or aarch64-zephyr-elf-gcc on PATH and no Zephyr SDK under ZEPHYR_SDK_INSTALL_DIR or \$CONNECTED_DEPS_DIR/zephyr-sdk-* (the uboot-mp2 qemu row would gap): Debian gcc-aarch64-linux-gnu, Gentoo cross-aarch64-linux-gnu, the Zephyr SDK, or set CROSS_COMPILE"
  fi
elif ! command -v "${cross}objcopy" > /dev/null && [ ! -x "${cross}objcopy" ]; then
  line missing aarch64-cross "${cross}gcc is there but ${cross}objcopy is not (run.sh makes the test images with it)"
else
  line ok aarch64-cross "$cross"
fi

# run.sh's host tools: PATH, then /usr/sbin and /sbin.
find_tool() {
  local t=$1 d p
  if p=$(command -v "$t"); then
    printf '%s' "$p"
    return 0
  fi
  for d in ${MP2_QEMU_CHECK_SBIN-/usr/sbin /sbin}; do
    if [ -x "$d/$t" ]; then
      printf '%s' "$d/$t"
      return 0
    fi
  done
  return 1
}

hint_python3="distro package python3"
hint_mke2fs="distro package e2fsprogs (also looked for in /usr/sbin and /sbin)"
hint_sfdisk="distro package fdisk (Debian) / sys-apps/util-linux (Gentoo) (also looked for in /usr/sbin and /sbin)"
hint_truncate="coreutils"
hint_sha256sum="coreutils"
hint_dd="coreutils"
for t in python3 mke2fs sfdisk truncate sha256sum dd; do
  if p=$(find_tool "$t"); then
    line ok "$t" "$p"
  else
    h=hint_$t
    line missing "$t" "not on PATH (the uboot-mp2 qemu row would gap): ${!h}"
  fi
done

if p=$(command -v git); then
  line ok git "$p"
else
  line missing git "not on PATH (run.sh's fetch check and build-uboot.sh's U-Boot fetch fail without it): distro package git"
fi
if p=$(command -v make); then
  line ok make "$p"
else
  line missing make "not on PATH (build-uboot.sh builds U-Boot with it on the first run of a host): distro package make"
fi

[ "$missing_count" -eq 0 ]
