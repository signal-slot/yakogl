#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Build the host decoder. Usage: build.sh [OUTPUT]  (default: $TMPDIR/decode)
# Prints the output path. CC overrides the compiler (default cc).
set -e
here=$(cd "$(dirname "$0")" && pwd)
hdr=$here/../../../../driver/zephyr/lib/board/rpi5_scanout
out=${1:-${TMPDIR:-/tmp}/decode}
${CC:-cc} -std=c99 -Wall -Wextra -Werror -O1 -I"$hdr" -o "$out" "$here/decode.c"
echo "$out"
