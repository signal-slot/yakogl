# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Test script of the loop-semantics cases (../README.md), not the product
# script. prefix-stock.cmd: the stock prefix of the snapshot case. Shows
# boot_prefixes as prefix-change.cmd left it and puts the original back (the
# harness checks boot_prefixes is unchanged after the boot).
echo TEST stock-prefix script runs, boot_prefixes is ${boot_prefixes}
setenv boot_prefixes ${mp2s_saved_prefixes}
setenv mp2s_saved_prefixes
