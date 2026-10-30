// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
pins! {
    CCB_CMD_TYPE_3D_PR: u32 = 1 => "RGXFWIF_CCB_CMD_TYPE_3D_PR";
    RTA_CTL_FLAGS: usize = 28 =>
        "offsetof(RGXFWIF_RTA_CTL, ui32RTACtlFlags)";
    PM_THING: u32 = 5 => "RGXFWIF_PM_THING";
    UNRELATED: u32 = 2 => "RGXFWIF_UNRELATED";
}
