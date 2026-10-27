#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Generate src/fw/mali_csffw_blob.h from the Mali-G610's CSF firmware image.
#
# The firmware binary is not redistributed in this repo (like the PowerVR one,
# tools/mkfwblob.sh). Take mali_csffw.bin from the NanoPC-T6's Armbian
# (package armbian-firmware), read only:
#
#   scp root@<board>:/lib/firmware/arm/mali/arch10.8/mali_csffw.bin .
#
# The board's file on 2026-10-26: 282624 bytes, sha256
# a27847ea11f8efb3136340c3ba8aab413ae25145eeb4a7f64ff5edd829a2405b (header 0.3,
# interface 1.5.0; driver/docs/nanopc-t6-mali.md, "The firmware (Z3)"). The
# gate reads the file at MALI_CSFFW (tools/cts/deps.toml); without it the
# firmware row is a gap. Then:
#
#   ./tools/mkmalifw.sh /path/to/mali_csffw.bin [out.h]
#
# Produces src/fw/mali_csffw_blob.h (gitignored, driver/.gitignore) with
# `const uint8_t mali_csffw_blob[]`, which the kernel side hands to
# gpu::mali::fw::parse (driver/rust). The image is checked for the CSF magic
# and header major 0 first, so a wrong file never becomes a header.
set -euo pipefail
IN="${1:?usage: mkmalifw.sh <mali_csffw.bin> [out.h]}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${2:-$HERE/src/fw/mali_csffw_blob.h}"
SZ=$(wc -c < "$IN")
# CSF_FW_BINARY_HEADER_MAGIC 0xc3f13a6e (little endian), then minor, major
HDR=$(od -An -v -tx1 -N6 "$IN" | tr -d ' \n')
[ "${HDR:0:8}" = 6e3af1c3 ] || { echo "mkmalifw.sh: $IN is not a CSF firmware image (magic ${HDR:0:8})" >&2; exit 1; }
[ "${HDR:10:2}" = 00 ] || { echo "mkmalifw.sh: $IN has header major 0x${HDR:10:2}; only 0 is known" >&2; exit 1; }
SHA=$(sha256sum < "$IN" | cut -c1-64)
mkdir -p "$(dirname "$OUT")"
{
  echo "/* Auto-generated from $(basename "$IN") by tools/mkmalifw.sh. Do not edit. */"
  echo "/* The Mali CSF firmware is Arm's and is NOT committed to this repo. */"
  echo "#ifndef MALI_CSFFW_BLOB_H"
  echo "#define MALI_CSFFW_BLOB_H"
  echo "#include <stdint.h>"
  echo "#define MALI_CSFFW_SIZE ${SZ}u"
  echo "#define MALI_CSFFW_SHA256 \"${SHA}\""
  echo "static const uint8_t mali_csffw_blob[MALI_CSFFW_SIZE] = {"
  od -An -v -tx1 "$IN" | sed -E 's/[0-9a-f]{2}/0x&,/g; s/^ *//'
  echo "};"
  echo "#endif /* MALI_CSFFW_BLOB_H */"
} > "$OUT"
echo "wrote $OUT ($SZ bytes of firmware, sha256 $SHA)"
