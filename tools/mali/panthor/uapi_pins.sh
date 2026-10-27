#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build uapi_pins.c against the two uAPI headers, run it, and compare its
# output with uapi_pins.expected (the pins src/uapi.rs tests include).
#
#   tools/mali/panthor/uapi_pins.sh <dir with Linux v6.18 panthor_drm.h> <dir with drm.h>
#
# The headers are not vendored: v6.18's include/uapi/drm/panthor_drm.h is at
# https://raw.githubusercontent.com/torvalds/linux/v6.18/include/uapi/drm/panthor_drm.h
# (Mesa's copy is newer: its timestamp_info differs). drm.h is the DRM core
# header (Mesa include/drm-uapi/drm.h or the kernel's).
# UAPI_PINS_UPDATE=1 rewrites uapi_pins.expected instead of comparing.
# Runs the aarch64 build under qemu-aarch64, then the host build too: both
# must agree.
set -euo pipefail
pdir=${1:?usage: $0 <dir with panthor_drm.h> <dir with drm.h>}
ddir=${2:?usage: $0 <dir with panthor_drm.h> <dir with drm.h>}
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
aarch64-linux-gnu-gcc -static -I"$pdir" -I"$ddir" -o "$tmp/a64" "$here/uapi_pins.c"
gcc -I"$pdir" -I"$ddir" -o "$tmp/x86" "$here/uapi_pins.c"
qemu-aarch64 "$tmp/a64" > "$tmp/a64.out"
"$tmp/x86" > "$tmp/x86.out"
[ -s "$tmp/a64.out" ] || { echo "uapi_pins.sh: the aarch64 run printed nothing" >&2; exit 1; }
cmp "$tmp/a64.out" "$tmp/x86.out" || { echo "uapi_pins.sh: aarch64 and x86_64 differ" >&2; exit 1; }
if [ "${UAPI_PINS_UPDATE-}" = 1 ]; then
  cp "$tmp/a64.out" "$here/uapi_pins.expected"
  echo "uapi_pins.sh: rewrote uapi_pins.expected ($(wc -l < "$tmp/a64.out") lines)"
else
  diff "$tmp/a64.out" "$here/uapi_pins.expected" && echo "uapi_pins.sh: $(wc -l < "$tmp/a64.out") pins match uapi_pins.expected"
fi
