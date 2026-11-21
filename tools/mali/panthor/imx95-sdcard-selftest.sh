#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/mali/panthor/imx95-sdcard.sh's pure parts
# (imx95_sdcard.py): the kernel fragment's check against a .config, the
# card's U-Boot script (its root PARTUUID, the Mallow dtb by U-Boot's
# ${variant}, the console) and its legacy script image (header, CRCs, a
# round trip, a corrupt byte refused), the card's partition layout, the
# firmware pin read from a deps.toml, and the script's argument handling.
# No kernel, rootfs, qemu or card.
#
#   tools/mali/panthor/imx95-sdcard-selftest.sh   prints "imx95-sdcard-selftest: N checks passed"
#
# Host only, under a second. Gate: a row of its own (tools/check_selftests.py
# lists it, as no row names it).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
PY=(python3 -I "$HERE/imx95_sdcard.py")
SH=$HERE/imx95-sdcard.sh
EXPECT=27
T=$(mktemp -d "${TMPDIR:-/tmp}/imx95-sdcard-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}
not() { ! "$@"; }
has() { grep -qF -- "$2" "$1"; }

# config-check
printf '# c\nCONFIG_A=y\nCONFIG_B=m\n# CONFIG_C is not set\nCONFIG_S="x y"\n' > "$T/frag"
printf 'CONFIG_A=y\nCONFIG_B=m\nCONFIG_S="x y"\nCONFIG_D=y\n' > "$T/good"
printf 'CONFIG_A=y\nCONFIG_B=y\nCONFIG_C=y\n' > "$T/bad"
check "config-check: a .config that holds every line passes" "${PY[@]}" config-check "$T/frag" "$T/good"
check "config-check: a changed, a set 'not set' and a missing line fail" not "${PY[@]}" config-check "$T/frag" "$T/bad"
"${PY[@]}" config-check "$T/frag" "$T/bad" > "$T/bad.out"
check "config-check: one line a miss, want and have named" \
  bash -c '[ "$(wc -l < "$1")" = 3 ] && grep -qx "config: CONFIG_B: want m, have y" "$1" && grep -qx "config: CONFIG_S: want \"x y\", have n" "$1"' _ "$T/bad.out"
check "the card's fragment: every line is a comment, blank or CONFIG_X=value" \
  bash -c '! grep -vE "^(#.*|$|CONFIG_[A-Z0-9_]+=[ymn0-9\"].*)$" "$1"' _ "$HERE/imx95-sdcard.config"
for s in CONFIG_DRM_PANTHOR=m CONFIG_ARM_SCMI_RAW_MODE_SUPPORT=y CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX=y CONFIG_DEBUG_FS=y CONFIG_KEXEC=y CONFIG_CRASH_DUMP=y CONFIG_NXP_ENETC4=m CONFIG_SERIAL_FSL_LPUART_CONSOLE=y CONFIG_MMC_SDHCI_ESDHC_IMX=y; do
  check "the card's fragment has $s" grep -qx "$s" "$HERE/imx95-sdcard.config"
done

# bootcmd and the script image
"${PY[@]}" bootcmd 1a95c0de > "$T/boot.cmd"
check "boot.cmd: root=PARTUUID=<id>-02 and the console ttyLP2,115200" \
  bash -c 'grep -q "root=PARTUUID=1a95c0de-02 " "$1" && grep -q "console=ttyLP2,115200" "$1"' _ "$T/boot.cmd"
check "boot.cmd: the Mallow dtb by \${variant}, wifi or nonwifi" \
  bash -c 'grep -qF "if test \"\${variant}\" = \"wifi\"; then setenv sd_fdt imx95-verdin-wifi-mallow.dtb; else setenv sd_fdt imx95-verdin-nonwifi-mallow.dtb; fi" "$1"' _ "$T/boot.cmd"
check "boot.cmd: booti Image and dtbs/\${sd_fdt} from \${devtype} \${devnum}:\${distro_bootpart}" \
  bash -c 'grep -qF "load \${devtype} \${devnum}:\${distro_bootpart} \${fdt_addr_r} dtbs/\${sd_fdt}; then booti \${kernel_addr_r} - \${fdt_addr_r}" "$1"' _ "$T/boot.cmd"
check "bootcmd: a PARTUUID prefix that is not 8 hex digits is refused" not "${PY[@]}" bootcmd 1A95C0DE
SOURCE_DATE_EPOCH=1793491200 "${PY[@]}" mkscr "$T/boot.cmd" "$T/boot.scr"
check "boot.scr: magic 27051956, time SOURCE_DATE_EPOCH, os linux, arch arm64, type script, no compression" \
  bash -c '[ "$(od -An -tx1 -N4 "$1" | tr -d " ")" = 27051956 ] && [ "$(od -An -tx1 -j8 -N4 "$1" | tr -d " ")" = 6ae68100 ] && [ "$(od -An -tu1 -j28 -N4 "$1" | tr -s " ")" = " 5 22 6 0" ]' _ "$T/boot.scr"
check "boot.scr: scr-dump gives boot.cmd back" bash -c '"$@" > "$0.txt" && cmp -s "$0.txt" "${0%.scr}.cmd"' "$T/boot.scr" "${PY[@]}" scr-dump "$T/boot.scr"
cp "$T/boot.scr" "$T/corrupt.scr"
printf 'X' | dd of="$T/corrupt.scr" bs=1 seek=100 conv=notrunc status=none
check "boot.scr: a changed byte of the script fails its data CRC" not "${PY[@]}" scr-dump "$T/corrupt.scr"
printf 'not an image at all, longer than a header: %064d' 0 > "$T/junk.scr"
check "scr-dump: a file without the magic is refused" not "${PY[@]}" scr-dump "$T/junk.scr"

# layout
"${PY[@]}" layout 0x1a95c0de 256 6144 > "$T/layout"
check "layout: dos, label-id 0x1a95c0de, p1 FAT32 (c) bootable 256 MiB at 1 MiB, p2 83 to the end" \
  bash -c 'grep -qx "label: dos" "$1" && grep -qx "label-id: 0x1a95c0de" "$1" && grep -qx "start=2048, size=524288, type=c, bootable" "$1" && grep -qx "start=526336, size=12056576, type=83" "$1"' _ "$T/layout"
check "layout: a root of less than 512 MiB is refused" not "${PY[@]}" layout 0x1a95c0de 256 700

# pin
printf '[[dep]]\nname = "MALI_CSFFW"\npath = "x"\nmarker = "x"\n\n[[dep.inputs]]\nurl = "https://example.invalid/f"\nsha256 = "%064d"\n' 7 > "$T/deps.toml"
check "pin: the first input's sha256 of the named entry" bash -c '[ "$("$@")" = "$(printf %064d 7)" ]' _ "${PY[@]}" pin "$T/deps.toml" MALI_CSFFW
check "pin: an entry that is not there fails" not "${PY[@]}" pin "$T/deps.toml" NOPE

# the script's arguments (nothing is built)
check "imx95-sdcard.sh: --help prints the steps" bash -c '"$1" --help | grep -q "verify     the card on the host"' _ "$SH"
check "imx95-sdcard.sh: an unknown step is refused (exit 2)" bash -c '"$1" --out "$2/o" frobnicate; [ $? = 2 ]' _ "$SH" "$T"

total=$((passed + failed))
if [ "$total" != "$EXPECT" ]; then
  echo "imx95-sdcard-selftest: ran $total checks, expected $EXPECT"
  exit 1
fi
if [ "$failed" != 0 ]; then
  echo "imx95-sdcard-selftest: $failed of $total checks FAILED"
  exit 1
fi
echo "imx95-sdcard-selftest: $passed checks passed"
