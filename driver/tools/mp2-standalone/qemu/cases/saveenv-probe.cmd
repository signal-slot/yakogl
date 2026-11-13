# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Test script of the loop-semantics cases (../README.md), not the product
# script. saveenv-probe.cmd: saveenv inside a sourced script. The first boot
# sets mp2s_probe and saves it; later boots report what they read.
if itest.s "${mp2s_probe}" == ""; then
	echo TEST probe unset
	setenv mp2s_probe saved-by-script
	saveenv
	echo TEST saveenv done
else
	echo TEST probe is ${mp2s_probe}
fi
exit
