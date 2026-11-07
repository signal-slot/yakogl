#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins tools/uboot_env.py on synthetic U-Boot environment images the test
# builds itself (no board data): a single copy and a redundant pair (flag
# selection, 0xff wrapping to 0x00, the inactive copy written), 0x00 and 0xff
# padding, a bad CRC and an overflow refused with exit 2 and no output file,
# and set then unset giving the input back byte for byte.
#
#   tools/uboot_env_selftest.sh
set -u
export LC_ALL=C LANG=C LANGUAGE=C
cd "$(dirname "$0")/.." || exit 2
exec python3 -I -B tools/testdata/uboot-env/test_uboot_env.py "$@"
