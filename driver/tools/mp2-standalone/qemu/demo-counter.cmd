# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# demo-counter.cmd: the harness's own demo script, not the product's. A
# counter in the saved environment: decrement mp2s_test, save it, print
# "boots left <n>". Run as /boot.scr.uimg by the distro boot (README.md).
if itest.s "${mp2s_test}" == ""; then
	echo mp2s: mp2s_test is not set
elif itest ${mp2s_test} > 0; then
	setexpr mp2s_test ${mp2s_test} - 1
	saveenv
	echo boots left ${mp2s_test}
else
	echo boots left 0
fi
