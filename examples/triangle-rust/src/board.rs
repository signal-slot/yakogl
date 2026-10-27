// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The board layer's calls this example makes, as
//! driver/zephyr/lib/board/include/yakogl_board.h declares them: the window,
//! the screen, the records and the end of the run. The header says what each
//! one does on every board.

use core::ffi::{c_char, c_int, c_uint, c_void};

/// `struct board_window`: a window the GPU may render into and the screen
/// may show.
#[repr(C)]
pub struct Window {
    pub phys: usize,
    /// The CPU's view: Normal non-cacheable on every board, so no cache
    /// maintenance.
    pub cpu: *mut c_void,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub format: u32,
}

extern "C" {
    pub fn board_window(w: u32, h: u32, out: *mut Window) -> c_int;
    pub fn board_show(win: *const Window) -> c_int;
    pub fn board_progress(where_: u32);
    pub fn board_result(kind: c_uint, w1: u32, w2: u32, w3: u32);
    pub fn board_gpu_resets() -> u32;
    pub fn board_finish(name: *const c_char, rc: c_int) -> !;
}
