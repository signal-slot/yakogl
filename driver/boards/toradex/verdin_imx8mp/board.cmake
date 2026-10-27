# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The A53 target's early GIC fix (undoing the EOI mode a kexec from Linux
# leaves behind) is no longer compiled here: it moved to the GPU driver's
# yakogl_gc_board library (driver/zephyr/lib/board/imx8mp/gc_board_gic.c), which
# every kexec'd i.MX8MP image links, whatever board describes the carrier.
