#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# kexec_run.sh - copy a Zephyr image to the board and boot it from Linux.
#
#   BOARD_HOST=torizon@<board> ./kexec_run.sh build/zephyr/zephyr.bin [kexec_zephyr]
#
# Waits until the board's boot has settled (systemd "running"/"degraded"):
# a kexec issued while Linux is still starting services sometimes produced a
# silent Zephyr that never reached its console. The files go to the user's
# home directory because /tmp is wiped on every reset.
#
# Environment:
#   BOARD_HOST       user@host (required)
#   BOARD_GPU_MODULES which kernel modules must release the GPU before the jump
#                    (default: pvrgpu pvrsrvkm gcgpu galcore -- both boards')
#   BOARD_LOAD_ADDR  where the image is linked: the loader's default
#                    (0x82000000, Verdin AM62P) when unset; 0x42000000 for
#                    the Verdin iMX8M Plus
#   BOARD_KEXEC_CRASH  set to 1 to take the kernel's crash route (the loader's
#                    --crash: no device shutdown, so the display stays on;
#                    needs a crashkernel= reservation holding the image)
#   BOARD_NO_WATCHDOG  set to 1 to pass --no-watchdog: jump even when the
#                    AM62P's RTI watchdog is not running (the loader refuses
#                    by default, because the image then has no way back).
#                    Only through the direct path; the scoped wrapper takes
#                    no such flag, so it always checks.
#   BOARD_SUDO_PASS  sudo password, piped to `sudo -S`; unset means
#                    passwordless sudo.
#                    Never put it in a script or a repository.
#
# With no password and the scoped wrapper installed by
# driver/tools/kexec-zephyr/board-sudo-setup.sh present on the board
# (/usr/local/sbin/kexec-zephyr), the files are staged in the wrapper's fixed
# directory and the jump is `sudo -n kexec-zephyr [addr] [--crash]`: no shell
# runs as root, the wrapper unloads the four GPU modules itself, and
# BOARD_GPU_MODULES is not consulted. This is how the i.MX8MP lab jumps.
set -eu
: "${BOARD_HOST:?set BOARD_HOST=user@host}"
IMG=${1:?zephyr.bin}
LOADER=${2:-$(dirname "$0")/kexec_zephyr}
# BOARD_SSH_OPTS: extra ssh/scp options, e.g. "-J <jump host>" when the board is
# only reachable through another machine. Empty by default.
SSH="ssh ${BOARD_SSH_OPTS:-} -o StrictHostKeyChecking=no -o ConnectTimeout=10"
SCP="scp ${BOARD_SSH_OPTS:-} -o StrictHostKeyChecking=no"

$SSH "$BOARD_HOST" 'systemctl is-system-running --wait >/dev/null 2>&1 || true; cat /proc/uptime'
CRASH=${BOARD_KEXEC_CRASH:+--crash}
# The crash route loads the image into the crashkernel= reservation, so an
# image whose RAM footprint is larger cannot jump. Find that out here, before
# anything on the board is touched: the loader only discovers it after the
# wrapper has unloaded the GPU modules (kexec_load: error 0x63, 2026-09-23,
# with gc_slint's ~95 MiB on the i.MX8MP's 64 MiB window). The footprint is
# the AArch64 image header's image_size (bytes 16..23, little-endian), which
# is what kexec_zephyr loads; the window is /sys/kernel/kexec_crash_size.
if [ -n "$CRASH" ]; then
  footprint=$(od -A n -t u8 -j 16 -N 8 "$IMG" | tr -d ' ')
  window=$($SSH "$BOARD_HOST" 'cat /sys/kernel/kexec_crash_size 2>/dev/null' </dev/null || true)
  if [ -n "$window" ] && [ -n "$footprint" ] && [ "$footprint" -gt "$window" ]; then
    echo "kexec_run.sh: REFUSED -- the image needs $footprint bytes and the crash window is $window;" >&2
    echo "              use the normal route (unset BOARD_KEXEC_CRASH). Nothing was staged or unloaded." >&2
    exit 3
  fi
fi
# Every jump is preceded by a sync in the same shell that jumps. Neither route
# flushes anything itself -- the crash route runs no shutdown at all -- so
# whatever the board wrote in the seconds before (a service's state, logs) is
# otherwise lost: on 2026-09-23 a Tailscale login made just before a jump did
# not survive it. The sync after staging, above, is too early for that.
# The scoped wrapper takes the jump when there is no password to type and the
# board has it; the two files then have to sit where the wrapper looks.
if [ -z "${BOARD_SUDO_PASS:-}" ] && $SSH "$BOARD_HOST" test -x /usr/local/sbin/kexec-zephyr </dev/null; then
  $SSH "$BOARD_HOST" 'mkdir -p ~/kexec'
  $SCP "$LOADER" "$BOARD_HOST":kexec/kexec_zephyr
  $SCP "$IMG" "$BOARD_HOST":kexec/zephyr.bin
  $SSH "$BOARD_HOST" sync
  echo "kexec_run.sh: jumping through /usr/local/sbin/kexec-zephyr ${BOARD_LOAD_ADDR:-} $CRASH"
  $SSH -o ServerAliveInterval=3 -o ServerAliveCountMax=2 "$BOARD_HOST" \
    "sync; sync; sudo -n /usr/local/sbin/kexec-zephyr ${BOARD_LOAD_ADDR:-} $CRASH" </dev/null || true
  exit 0
fi
$SCP "$LOADER" "$IMG" "$BOARD_HOST":
$SSH "$BOARD_HOST" sync
# The GPU driver must release the GPU before Zephyr takes it, and WHICH driver
# that is depends on the board. The list used to be the AM62P's alone --
# `pvrgpu` (our harness) and `pvrsrvkm` (TI's stock) -- so on the i.MX8MP, whose
# driver is `gcgpu`, the jump left our own module loaded and holding the GPU.
# Nothing in the output would have said so: rmmod of an absent module is
# silenced, and the session ends at the jump either way.
#
# BOARD_GPU_MODULES names them, defaulting to all four so an operator who does
# not set it gets the old behaviour plus the Vivante one. Unloading a module
# that is not loaded is free; not unloading one that is costs the boot.
#
# The ssh session ends when the kernel jumps to Zephyr.
# With a password, sudo reads it from stdin, which ssh forwards: it appears in
# no command line and no log. Without one, stdin is closed so a sudo that does
# ask fails at once instead of hanging.
RMMODS=""
for m in ${BOARD_GPU_MODULES:-pvrgpu pvrsrvkm gcgpu galcore}; do
  RMMODS="$RMMODS rmmod $m 2>/dev/null;"
done
NOWDT=${BOARD_NO_WATCHDOG:+--no-watchdog}
JUMP="$RMMODS ./$(basename "$LOADER") ./$(basename "$IMG") ${BOARD_LOAD_ADDR:-} $CRASH $NOWDT"
if [ -n "${BOARD_SUDO_PASS:-}" ]; then
  printf '%s\n' "$BOARD_SUDO_PASS" | $SSH -o ServerAliveInterval=3 -o ServerAliveCountMax=2 "$BOARD_HOST" \
    "sync; sync; sudo -S -p '' sh -c '$JUMP'" || true
else
  $SSH -o ServerAliveInterval=3 -o ServerAliveCountMax=2 "$BOARD_HOST" \
    "sync; sync; sudo -n sh -c '$JUMP'" </dev/null || true
fi
