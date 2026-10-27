// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How long an i.MX8MP sample stays up after its verdict before it resets
//! the SoC (driver/zephyr/lib/board/imx8mp/gc_sample.c, the samples' end).
//!
//! The hold was once the only readout: a board with no console said its
//! verdict by when it came back (60 s PASS, 40 s FAIL, 70 s + step * code).
//! The SNVS result record carries the verdict now, so a sample that wrote
//! its record ends at once ([`Hold::recorded`]). A coded hold stays only for
//! what no record field carries: a code a diagnostic claimed, and the GPU
//! reset count.

/// What the end of a sample knows (`struct yd_imx8mp_hold`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    /// 1 when the verdict is already in the result record
    /// (gc_sample_finish_recorded), 0 for gc_sample_finish.
    pub recorded: u32,
    /// 1 when the verdict was FAIL.
    pub failed: u32,
    /// The sample's own code, 0..=63, or -1 when it claimed none.
    pub code: i32,
    /// GPU resets during the run (latched at the verdict).
    pub gpu_resets: u32,
    /// CONFIG_GC_SAMPLE_HOLD_SECONDS and _FAIL: the uncoded holds.
    pub pass_s: u32,
    pub fail_s: u32,
    /// CONFIG_GC_SAMPLE_HOLD_CODE_STEP.
    pub step_s: u32,
    /// The codes that say "the GPU was reset N times": base + N - 1, N at
    /// most `reset_max` (GC_SAMPLE_HOLD_GPU_RESET_BASE / _MAX).
    pub reset_base: u32,
    pub reset_max: u32,
}

/// A coded hold is this plus `step_s` per unit of the code.
pub const CODE_BASE_S: u32 = 70;

/// The hold: `(seconds, code)`, the code -1 when the hold carries none.
pub fn hold_seconds(h: &Hold) -> (u32, i32) {
    let mut code = h.code;
    if code < 0 && h.gpu_resets != 0 {
        code = (h.reset_base + h.gpu_resets.min(h.reset_max) - 1) as i32;
    }
    if code >= 0 {
        (CODE_BASE_S + h.step_s * code as u32, code)
    } else if h.recorded != 0 {
        (0, -1)
    } else if h.failed != 0 {
        (h.fail_s, -1)
    } else {
        (h.pass_s, -1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Hold = Hold {
        recorded: 1,
        failed: 0,
        code: -1,
        gpu_resets: 0,
        pass_s: 60,
        fail_s: 40,
        step_s: 6,
        reset_base: 56,
        reset_max: 8,
    };

    #[test]
    fn a_recorded_end_resets_at_once_pass_or_fail() {
        assert_eq!(hold_seconds(&BASE), (0, -1));
        assert_eq!(hold_seconds(&Hold { failed: 1, ..BASE }), (0, -1));
    }

    #[test]
    fn the_older_end_keeps_the_pass_and_fail_holds() {
        assert_eq!(hold_seconds(&Hold { recorded: 0, ..BASE }), (60, -1));
        assert_eq!(hold_seconds(&Hold { recorded: 0, failed: 1, ..BASE }), (40, -1));
    }

    #[test]
    fn a_claimed_code_is_held_on_either_end() {
        // the regression set's old pass count, 34: 70 + 6 * 34 = 274 s
        // (the 2026-10-04 kd runs: settle 100 s + 274 s, back at ~400 s)
        assert_eq!(hold_seconds(&Hold { code: 34, ..BASE }), (274, 34));
        assert_eq!(hold_seconds(&Hold { recorded: 0, code: 0, ..BASE }), (70, 0));
    }

    #[test]
    fn gpu_resets_without_a_code_are_the_reset_codes() {
        assert_eq!(hold_seconds(&Hold { gpu_resets: 1, ..BASE }), (70 + 6 * 56, 56));
        assert_eq!(hold_seconds(&Hold { gpu_resets: 30, ..BASE }), (70 + 6 * 63, 63), "N clamps at 8");
        assert_eq!(hold_seconds(&Hold { gpu_resets: 3, code: 5, ..BASE }), (100, 5), "a claimed code wins");
    }
}
