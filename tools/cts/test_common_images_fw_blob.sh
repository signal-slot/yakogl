#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of common-images.sh's PowerVR firmware header step (--fw-blob-only):
# the copy from PVR_FW_BLOB, no overwrite, the error with nothing, and other
# boards left alone. Runs in a temporary directory; needs no Zephyr.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/fwblob.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/tools/cts"
cp "$HERE/common-images.sh" "$T/tools/cts/"
S=$T/tools/cts/common-images.sh
H=$T/driver/src/fw/pvr_fw_blob.h
fail=0
check() { if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
echo fake-new > "$T/blob.h"
echo fake-old > "$T/other.h"

env -u PVR_FW_BLOB bash "$S" --fw-blob-only am62p 2> "$T/err"; rc=$?
check "unset, no header: exit 2" "$([ $rc = 2 ]; echo $?)"
check "error names PVR_FW_BLOB" "$(grep -q PVR_FW_BLOB "$T/err"; echo $?)"
check "no header made" "$([ ! -e "$H" ]; echo $?)"

PVR_FW_BLOB=$T/missing bash "$S" --fw-blob-only am62p 2> /dev/null; rc=$?
check "unreadable file: exit 2" "$([ $rc = 2 ]; echo $?)"

PVR_FW_BLOB=$T/blob.h bash "$S" --fw-blob-only pi4; rc=$?
check "pi4 leaves it alone" "$([ $rc = 0 ] && [ ! -e "$H" ]; echo $?)"

PVR_FW_BLOB=$T/blob.h bash "$S" --fw-blob-only am62p; rc=$?
check "copy" "$([ $rc = 0 ] && cmp -s "$T/blob.h" "$H"; echo $?)"

PVR_FW_BLOB=$T/other.h bash "$S" --fw-blob-only am62p; rc=$?
check "existing header kept" "$([ $rc = 0 ] && cmp -s "$T/blob.h" "$H"; echo $?)"

env -u PVR_FW_BLOB bash "$S" --fw-blob-only am62p; rc=$?
check "existing header, no variable: ok" "$([ $rc = 0 ]; echo $?)"
exit $fail
