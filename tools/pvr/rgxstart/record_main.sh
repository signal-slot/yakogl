#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# record_main.sh <workdir>: rebuild tools/pvr/rgxstart/main-38438f4d.trace, the
# register sequence of the firmware start before docs/source-form.md step 6
# (record_main.c says how). Needs the commit in the repository's history; the
# trace itself is committed, so the check (tools/check.sh) does not.
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
OUT=$1
mkdir -p "$OUT"
git -C "$REPO" show 38438f4d:driver/src/pvr_fw_boot.c > "$OUT/pvr_fw_boot.c"
# From the register-init banner to the line before pvr_wait_for_fw_boot.
awk '/^\/\* ---- register init sequence/ {on = 1} /^static int pvr_wait_for_fw_boot/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_start.inc"
# shellcheck disable=SC2046
cc -std=gnu11 -Wall -Wno-unused-function -DTEE_DDK $("$REPO/driver/tools/offsetof/ddk-cflags.sh") \
	-I"$REPO/tools/pvr/rgxstart/hostshim" -I"$REPO/driver/src" -I"$OUT" \
	"$REPO/tools/pvr/rgxstart/record_main.c" -o "$OUT/record_main"
"$OUT/record_main" > "$OUT/main-38438f4d.trace"
echo "$OUT/main-38438f4d.trace"
