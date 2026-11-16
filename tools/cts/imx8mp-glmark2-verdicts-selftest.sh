#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-verdicts-selftest.sh
#
# Host-only: the checks of tools/cts/imx8mp-glmark2-verdicts.py on the
# fixtures in tools/cts/testdata/imx8mp-glmark2-verdicts/.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
exec python3 -I "$HERE/imx8mp-glmark2-verdicts.py" --self-test
