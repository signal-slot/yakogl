#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# g310_check.sh - is every deqp_*.expect.json reference identical on the
# Mali-G310v2 and the Mali-G610? Draws each reference's shader pair on Mesa
# 26.2.1's unmodified drm-shim for both GPUs and compares the extracts.
# See "Checking the G310" in tools/mali/README-deqp_ref.md.
#
#   tools/mali/g310_check.sh [--since <git rev>] [--build-dir <dir>] [--scratch <dir>] [<glob or name>...]
set -euo pipefail
exec python3 -I "$(cd "$(dirname "$0")" && pwd)/g310_check.py" "$@"
