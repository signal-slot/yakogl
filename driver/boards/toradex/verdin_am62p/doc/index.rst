.. SPDX-License-Identifier: Apache-2.0
.. SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

.. _verdin_am62p:

Toradex Verdin AM62P
####################

Overview
********

Board support for the Toradex Verdin AM62P SoM (TI AM62P54: 4x Cortex-A53,
MCU Cortex-R5F, IMG BXS GPU) on the Dahlia carrier board, as used by the
SafeUI application platform PoC.  This tree provides the Cortex-A53 target:

* ``verdin_am62p/am62p54/a53`` — Cortex-A53 cluster

The MCU-domain Cortex-R5F target (``mcu_r5f0_0``) stays with the SafeUI
platform firmware and is not part of this tree.

Status
******

Used on silicon by this repository's samples (``driver/samples/zephyr``),
loaded by U-Boot (``go 0x82000000``) or kexec'd from Linux.
