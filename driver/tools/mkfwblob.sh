#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Generate src/fw/pvr_fw_blob.h from a PowerVR firmware image.
#
# The firmware binary is not redistributed in this repo. Take rgx.fw from the
# board's /lib/firmware (BVNC 36.53.104.796) or from the matching upstream
# linux-firmware / Mesa open-firmware package, then:
#
#   ./tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796
#
# Produces src/fw/pvr_fw_blob.h with `const uint8_t pvr_fw_blob[]` used by
# src/pvr_fw_boot.c.
set -euo pipefail
IN="${1:?usage: mkfwblob.sh <firmware-file>}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/src/fw/pvr_fw_blob.h"
mkdir -p "$(dirname "$OUT")"
SZ=$(wc -c < "$IN")
{
  echo "/* Auto-generated from $(basename "$IN") by tools/mkfwblob.sh. Do not edit. */"
  echo "/* PowerVR firmware is TI/Img property and is NOT committed to this repo. */"
  echo "#ifndef PVR_FW_BLOB_H"
  echo "#define PVR_FW_BLOB_H"
  echo "#define PVR_FW_BLOB_SIZE ${SZ}u"
  echo "static const uint8_t pvr_fw_blob[PVR_FW_BLOB_SIZE] = {"
  od -An -v -tx1 "$IN" | sed -E 's/[0-9a-f]{2}/0x&,/g; s/^ *//'
  echo "};"
  echo "#endif /* PVR_FW_BLOB_H */"
} > "$OUT"
echo "wrote $OUT ($(wc -c < "$IN") bytes of firmware)"
