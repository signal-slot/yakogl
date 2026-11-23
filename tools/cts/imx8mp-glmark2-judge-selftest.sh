#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-judge-selftest.sh
#
# Host-only, no board: tools/cts/imx8mp-glmark2-judge.py on the synthetic
# record and reference of tools/cts/testdata/imx8mp-glmark2-judge/, in $TMPDIR.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
exec python3 -I "$HERE/imx8mp-glmark2-judge.py" --self-test
