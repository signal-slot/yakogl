#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3_rows.py (row specs of the AM62P not-pass TSV header).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec python3 -I "$here/gles3_rows.py" --self-test
