#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# uboot_env_selftest.sh - tools/uboot_env.py on synthetic environment images
# (built by the test itself; no board data).
#
#   tools/uboot_env_selftest.sh
set -u
cd "$(dirname "$0")/.." || exit 2
exec python3 -I tools/testdata/uboot-env/test_uboot_env.py "$@"
