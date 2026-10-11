// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The KICK_DIFF comparison (src/vivante/kick.rs `diff`): two reads of the
//! render target after a kick, to tell a late write from a wrong one.
//!
//!   cargo test --no-default-features --features host-test,gpu-vivante,test-knobs --test gc_kick_diff

#![cfg(all(feature = "gpu-vivante", feature = "test-knobs"))]

use std::sync::atomic::{AtomicPtr, Ordering};
use yakogl::vivante::kick::diff::*;

const CLEAR: u32 = 0xff00_0000;
const RED: u32 = 0xff00_00ff;

#[test]
fn equal_reads_are_stable() {
    let a = vec![CLEAR; 64];
    let d = compare(&a, &a, 16);
    assert_eq!(d.verdict(), Verdict::Stable);
    assert_eq!((d.changed, d.rows, d.first, d.last), (0, 0, 64, 0));
}

#[test]
fn a_late_pixel_is_found_with_its_values() {
    let first = vec![CLEAR; 64];
    let mut second = first.clone();
    second[21] = RED;
    let d = compare(&first, &second, 16);
    assert_eq!(d.verdict(), Verdict::Late);
    assert_eq!((d.changed, d.rows, d.first, d.last), (1, 1, 21, 21));
    assert_eq!((d.first_before, d.first_after), (CLEAR, RED));
}

#[test]
fn rows_count_once_each_and_in_order() {
    let first = vec![CLEAR; 64];
    let mut second = first.clone();
    // Two words in row 0, one in row 2 (rows of 16 words).
    for i in [3, 9, 33] {
        second[i] = RED;
    }
    let d = compare(&first, &second, 16);
    assert_eq!((d.changed, d.rows, d.first, d.last), (3, 2, 3, 33));
}

#[test]
fn zero_pitch_is_one_row() {
    let first = vec![CLEAR; 8];
    let mut second = first.clone();
    second[0] = RED;
    second[7] = RED;
    assert_eq!(compare(&first, &second, 0).rows, 1);
}

#[test]
fn the_shorter_read_sets_the_length() {
    let first = vec![CLEAR; 8];
    let mut second = vec![CLEAR; 4];
    second[3] = RED;
    let d = compare(&first, &second, 4);
    assert_eq!((d.words, d.changed, d.first), (4, 1, 3));
}

#[test]
fn a_write_that_never_lands_is_stable() {
    // Both reads miss the pixel: the instrument says "stable", and the doc
    // says the case's own check then tells it from a right target.
    let first = vec![CLEAR; 16];
    assert_eq!(compare(&first, &first.clone(), 4).verdict(), Verdict::Stable);
}

static TARGET: AtomicPtr<u32> = AtomicPtr::new(std::ptr::null_mut());

/// The harness's delay: the "GPU" lands its write while the CPU waits.
unsafe extern "C" fn land_late_write(_us: u32) {
    unsafe { TARGET.load(Ordering::Relaxed).add(5).write(RED) };
}

/// One test for the armed path: the knob state is global.
#[test]
fn armed_path_reports_and_counts() {
    let mut mem = vec![CLEAR; 32];
    TARGET.store(mem.as_mut_ptr(), Ordering::Relaxed);

    // Off until armed.
    assert_eq!(gc_kick_diff_after_fence(), 0);

    unsafe { gc_kick_diff_enable(mem.as_ptr(), 32, 8, 10_000, Some(land_late_write)) };
    assert_eq!(gc_kick_diff_after_fence(), 2, "the write landed during the delay");
    assert_eq!(gc_kick_diff_after_fence(), 1, "the pixel is already there at the next first read");

    let mut stats = [0u32; 3];
    unsafe { gc_kick_diff_stats(stats.as_mut_ptr()) };
    assert_eq!(stats, [2, 1, 1]);

    // Re-arming resets the counters; a null pointer disarms.
    unsafe { gc_kick_diff_enable(std::ptr::null(), 0, 0, 0, None) };
    assert_eq!(gc_kick_diff_after_fence(), 0);
    unsafe { gc_kick_diff_stats(stats.as_mut_ptr()) };
    assert_eq!(stats, [0, 0, 0]);
    TARGET.store(std::ptr::null_mut(), Ordering::Relaxed);
}
