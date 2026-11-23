#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2_uboot_session_preflight.sh on a fake tree and a fake PATH.
set -u
here=$(cd "$(dirname "$0")" && pwd)
p="$here/mp2_uboot_session_preflight.sh"
t=$(mktemp -d "${TMPDIR:-/tmp}/mp2usp.XXXXXX")
trap 'rm -rf "$t"' EXIT
ok=0
bad=0
chk() { if [ "$2" = 0 ]; then ok=$((ok + 1)); echo "ok: $1"; else bad=$((bad + 1)); echo "FAIL: $1"; fi; }

root=$t/root
mkdir -p "$root/tools" "$root/driver/tools/uboot-mp2" "$root/docs/notes" "$t/bin"
for f in tools/mp2_uboot_console.py tools/tftp_oneshot.py tools/uboot-netboot-lib.sh \
  driver/tools/uboot-mp2/mp2-mmc.cmd driver/tools/uboot-mp2/mp2-net.cmd \
  docs/notes/stm32mp257f-dk-uboot-boot.md; do : > "$root/$f"; done
for f in tools/board-stm32mp2-uboot.sh driver/tools/uboot-mp2/build.sh; do
  : > "$root/$f"
  chmod +x "$root/$f"
done
for c in python3 ssh mkimage; do
  printf '#!/bin/sh\n' > "$t/bin/$c"
  chmod +x "$t/bin/$c"
done
base=$(dirname "$(command -v dirname)")
run() { env -i PATH="$t/bin:$base" MP2_PREFLIGHT_ROOT="$root" "$@" "$p" 2>&1; }

out=$(run BOARD_HOST=hostval BOARD_UART=uartval)
rc=$?
chk ready "$rc"
echo "$out" | tail -1 | grep -q '^mp2_uboot_session_preflight: ready$'
chk ready-line $?
n=$(echo "$out" | grep -c '^preflight: ')
echo "$out" | grep -q "^mp2_uboot_session_preflight: $n ok, 0 failed$"
chk count-matches-lines $?
echo "$out" | grep -q 'BOARD_HOST set'
chk env-reported-as-set $?
if echo "$out" | grep -qE 'hostval|uartval'; then chk values-not-printed 1; else chk values-not-printed 0; fi

out=$(run BOARD_UART=uartval)
rc=$?
[ "$rc" = 1 ]
chk missing-host-rc $?
echo "$out" | grep -q 'preflight: env-host: FAIL BOARD_HOST not set'
chk missing-host-named $?
echo "$out" | tail -1 | grep -q 'not ready'
chk not-ready-line $?

rm "$root/driver/tools/uboot-mp2/mp2-net.cmd"
out=$(run BOARD_HOST=h BOARD_UART=u)
rc=$?
[ "$rc" = 1 ]
chk missing-script-rc $?
echo "$out" | grep -q 'script-net: FAIL missing'
chk missing-script-named $?
: > "$root/driver/tools/uboot-mp2/mp2-net.cmd"

rm "$t/bin/mkimage"
out=$(run BOARD_HOST=h BOARD_UART=u)
rc=$?
[ "$rc" = 1 ]
chk missing-mkimage-rc $?
echo "$out" | grep -q 'mkimage: FAIL'
chk missing-mkimage-named $?
printf '#!/bin/sh\n' > "$t/bin/mkimage"
chmod +x "$t/bin/mkimage"

out=$(run BOARD_HOST=h BOARD_UART=u MP2_NO_WATCHDOG=1)
rc=$?
[ "$rc" = 1 ]
chk no-watchdog-refused $?

echo 'fw_setenv bootcmd x' > "$root/tools/board-stm32mp2-uboot.sh"
out=$(run BOARD_HOST=h BOARD_UART=u)
rc=$?
[ "$rc" = 1 ]
chk bootcmd-write-refused $?

echo "mp2_uboot_session_preflight_selftest: $ok ok, $bad failed"
[ "$bad" = 0 ]
