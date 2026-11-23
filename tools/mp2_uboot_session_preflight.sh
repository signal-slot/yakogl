#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# One host check before a STM32MP257F-DK U-Boot one-shot session
# (docs/notes/stm32mp257f-dk-uboot-boot.md): are the runner, its helpers, the
# U-Boot script sources and the environment the run reads all present?
#
#   tools/mp2_uboot_session_preflight.sh
#
# Read-only and local: no board, no network, no sudo, no file written. The
# environment is reported only as "set" or "not set", never the value.
# Output: one "preflight: <check>: ok|FAIL <why>" line per check, then
# "mp2_uboot_session_preflight: <n> ok, <m> failed" and, last, "ready" or
# "not ready". Exit 0 only when every check is ok.
# Test override (selftest): MP2_PREFLIGHT_ROOT=<tree laid out like the repo>.
set -u
root=${MP2_PREFLIGHT_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
ok=0
bad=0
res() { # name status [why]
  if [ "$2" = ok ]; then
    ok=$((ok + 1))
    echo "preflight: $1: ok ${3:-}"
  else
    bad=$((bad + 1))
    echo "preflight: $1: FAIL ${3:-}"
  fi
}
have_file() { if [ -f "$root/$2" ]; then res "$1" ok; else res "$1" FAIL "missing $2"; fi; }
have_exec() { if [ -x "$root/$2" ]; then res "$1" ok; else res "$1" FAIL "not executable: $2"; fi; }
have_cmd() { if command -v "$2" >/dev/null 2>&1; then res "$1" ok; else res "$1" FAIL "$2 not on PATH"; fi; }
env_set() { if [ -n "${!2:-}" ]; then res "$1" ok "$2 set"; else res "$1" FAIL "$2 not set"; fi; }

# files
have_exec runner tools/board-stm32mp2-uboot.sh
have_file console tools/mp2_uboot_console.py
have_file tftp-server tools/tftp_oneshot.py
have_file netboot-lib tools/uboot-netboot-lib.sh
have_file script-mmc driver/tools/uboot-mp2/mp2-mmc.cmd
have_file script-net driver/tools/uboot-mp2/mp2-net.cmd
have_exec script-build driver/tools/uboot-mp2/build.sh
have_file note docs/notes/stm32mp257f-dk-uboot-boot.md
# host tools
have_cmd python3 python3
have_cmd ssh ssh
have_cmd mkimage mkimage
# environment (names only; values never printed)
env_set env-host BOARD_HOST
env_set env-uart BOARD_UART
# the stored bootcmd must never be a target of a persistent write
if [ -f "$root/tools/board-stm32mp2-uboot.sh" ] &&
   grep -Eq '(fw_setenv|saveenv|env save)[^#]*bootcmd' "$root/tools/board-stm32mp2-uboot.sh"; then
  res bootcmd-untouched FAIL "runner writes bootcmd persistently"
else
  res bootcmd-untouched ok
fi
# the watchdog is not switched off by default
if [ -n "${MP2_NO_WATCHDOG:-}" ]; then
  res watchdog-armed FAIL "MP2_NO_WATCHDOG set"
else
  res watchdog-armed ok
fi

echo "mp2_uboot_session_preflight: $ok ok, $bad failed"
if [ "$bad" = 0 ]; then
  echo "mp2_uboot_session_preflight: ready"
  exit 0
fi
echo "mp2_uboot_session_preflight: not ready"
exit 1
