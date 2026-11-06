// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Firmware interfaces a board's secure side serves: [`scmi`], the Arm System
//! Control and Management Interface codec (Arm DEN0056). No SoC ids and no
//! OS glue live here; a board module supplies the ids and a transport.

pub mod scmi;
