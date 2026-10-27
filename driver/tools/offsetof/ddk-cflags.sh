#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Print the compiler flags a host build needs to compile the vendored TI/Img
# DDK 25.2 firmware-interface headers the way the AM62P firmware was built:
# the include directories of third_party/ti-img-rogue-ddk-25.2 and its
# generated configuration, force-included. Used by ddk_abi_check.c's row in
# tools/check.sh.
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
DDK=$REPO/third_party/ti-img-rogue-ddk-25.2
printf '%s ' \
	"-I$DDK/config/am62p_linux_release" "-include" "config_kernel.h" \
	"-I$DDK/include" "-I$DDK/include/rogue" "-I$DDK/include/public" \
	"-I$DDK/hwdefs/rogue" "-I$DDK/hwdefs/rogue/km" \
	"-I$DDK/services/include" "-I$DDK/services/server/include" \
	"-I$DDK/services/server/devices" \
	"-I$REPO/driver/tools/offsetof" "-I$REPO/driver/compat"
echo
