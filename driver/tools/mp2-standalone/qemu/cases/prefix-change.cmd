# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Test script of the loop-semantics cases (../README.md), not the product
# script. prefix-change.cmd: switch boot_prefixes to a list the loop is not
# walking (/new/), then exit. prefix-stock.cmd puts the original back.
echo TEST first-prefix script runs, boot_prefixes was ${boot_prefixes}
setenv mp2s_saved_prefixes ${boot_prefixes}
setenv boot_prefixes /new/
echo TEST boot_prefixes now ${boot_prefixes}
exit
