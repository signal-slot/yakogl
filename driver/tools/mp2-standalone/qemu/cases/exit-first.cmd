# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Test script of the loop-semantics cases (../README.md), not the product
# script. exit-first.cmd: print a marker, call exit; the last line must never
# run.
echo TEST first-prefix script runs
exit
echo TEST after-exit line ran
