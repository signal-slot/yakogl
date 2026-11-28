#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of tools/cts/mrt4-verdict.py: runs its unittest.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec python3 -I "$here/test_mrt4_verdict.py"
