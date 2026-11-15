#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# edid: driver/zephyr/lib/board/rpi5_scanout/edid.h against hand-built EDID
# blocks (driver/zephyr/lib/board/test/edid_mock.c), built with the host cc,
# -std=c99 -Wall -Wextra -Werror.
#
# Run from anywhere. Builds into a mktemp dir under $TMPDIR. The last line is
# "N of N checks, 0 failed" and the exit 0 when all pass; exit 1 otherwise;
# exit 77 with "gap: CC" when there is no C compiler.
set -u
cd "$(dirname "$0")/../../.." || exit 1

cc=${CC:-cc}
src=driver/zephyr/lib/board/test/edid_mock.c

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
for f in $src driver/zephyr/lib/board/rpi5_scanout/edid.h; do
  [ -f "$f" ] || { echo "edid: $f missing, 1 failed"; exit 1; }
done

out=$(mktemp -d "${TMPDIR:-/tmp}/edid_mock.XXXXXX") || exit 1
trap 'rm -rf "$out"' EXIT

"$cc" -std=c99 -g -O1 -Wall -Wextra -Werror "$src" -o "$out/edid_mock" || {
  echo "edid: build failed, 1 failed"
  exit 1
}
"$out/edid_mock"
exit $?
