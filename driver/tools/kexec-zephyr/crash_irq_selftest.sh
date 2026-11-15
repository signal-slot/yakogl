#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# crash_irq.h without a board: kexec_zephyr's steps before the crash route's
# panic (driver/docs/nanopc-t6-display.md, section 2(a)). The host C compiler
# builds crash_irq_test.c, which includes crash_irq.h with made-up /sys and
# /proc reads and writes, and runs it on the NanoPC-T6's /proc/interrupts in
# testdata/t6-proc-interrupts.txt. Every check it makes is printed, and its
# last line counts them.
#
#   sh driver/tools/kexec-zephyr/crash_irq_selftest.sh
#
# The gate runs it from board_lock_selftest.sh.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
${CC:-cc} -std=c11 -O1 -Wall -Wextra -Werror -o "$tmp/crash_irq_test" "$here/crash_irq_test.c"
"$tmp/crash_irq_test" "$here/testdata/t6-proc-interrupts.txt"
