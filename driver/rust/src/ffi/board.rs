// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The record writer for the board layer's C sinks
//! (driver/zephyr/lib/board/common/record.c): the rules of
//! [`crate::board::record::Writer`] behind `yd_record_*`. The C side keeps
//! the writer (`struct yd_record_writer`, include/yakogl_drivers.h), calls
//! one of these, and stores `out` in its sink when the call returns 1.

use crate::board::record::{self, Glmark2Outcome, Writer};
use crate::board::{imx8mp, rpi4};
use core::mem::{offset_of, size_of};

const _: () = assert!(size_of::<Writer>() == 8);
const _: () = assert!(offset_of!(Writer, seq) == 0);
const _: () = assert!(offset_of!(Writer, have_result) == 1);
const _: () = assert!(offset_of!(Writer, progress_count) == 4);

/// # Safety
/// `w` and `out` are valid, aligned and not aliased for the call.
unsafe fn emit(w: *mut Writer, out: *mut [u32; 4], f: impl FnOnce(&mut Writer) -> Option<[u32; 4]>) -> i32 {
    // SAFETY: the caller's contract.
    let (w, out) = unsafe { (&mut *w, &mut *out) };
    match f(w) {
        Some(r) => {
            *out = r;
            1
        }
        None => 0,
    }
}

/// A result record with the next sequence number; always 1.
///
/// # Safety
/// `w` and `out` point to a writer and four words the caller owns.
#[no_mangle]
pub unsafe extern "C" fn yd_record_result(
    w: *mut Writer,
    kind: u32,
    w1: u32,
    w2: u32,
    w3: u32,
    out: *mut [u32; 4],
) -> i32 {
    // SAFETY: forwarded.
    unsafe { emit(w, out, |w| Some(w.result(kind as u8, w1, w2, w3))) }
}

/// A result record with `tag` in the low byte; always 1.
///
/// # Safety
/// As [`yd_record_result`].
#[no_mangle]
pub unsafe extern "C" fn yd_record_result_tag(
    w: *mut Writer,
    kind: u32,
    tag: u32,
    w1: u32,
    w2: u32,
    w3: u32,
    out: *mut [u32; 4],
) -> i32 {
    // SAFETY: forwarded.
    unsafe { emit(w, out, |w| Some(w.result_tag(kind as u8, tag as u8, w1, w2, w3))) }
}

/// A PROGRESS breadcrumb: 1 to store, 0 once a result was stored.
///
/// # Safety
/// As [`yd_record_result`].
#[no_mangle]
pub unsafe extern "C" fn yd_record_progress(w: *mut Writer, where_: u32, uptime_ms: u32, out: *mut [u32; 4]) -> i32 {
    // SAFETY: forwarded.
    unsafe { emit(w, out, |w| w.progress(where_, uptime_ms)) }
}

/// A breadcrumb of another kind (`tag` in the low byte): 1 to store, 0 once
/// a result was stored.
///
/// # Safety
/// As [`yd_record_result`].
#[no_mangle]
pub unsafe extern "C" fn yd_record_breadcrumb(
    w: *mut Writer,
    kind: u32,
    tag: u32,
    w1: u32,
    w2: u32,
    w3: u32,
    out: *mut [u32; 4],
) -> i32 {
    // SAFETY: forwarded.
    unsafe { emit(w, out, |w| w.breadcrumb(kind as u8, tag as u8, w1, w2, w3)) }
}

// ---- kind GLMARK2 (driver/samples/zephyr/glmark2) ---------------------------

const _: () = assert!(size_of::<Glmark2Outcome>() == 48);
const _: () = assert!(offset_of!(Glmark2Outcome, failed_entry) == 24);
const _: () = assert!(offset_of!(Glmark2Outcome, unsupported_entries) == 32);
const _: () = assert!(offset_of!(Glmark2Outcome, h) == 44);

/// kind GLMARK2's w1..w3 for a run's outcome ([`record::glmark2_words`]).
///
/// # Safety
/// `o` is readable and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn yd_record_glmark2_words(o: *const Glmark2Outcome, out: *mut [u32; 3]) {
    // SAFETY: the caller's contract.
    unsafe { *out = record::glmark2_words(&*o) };
}

/// The GLMARK2_WHY_* of one logged line of `n` bytes ([`record::glmark2_why`]).
///
/// # Safety
/// `line` is readable for `n` bytes, or `n` is 0.
#[no_mangle]
pub unsafe extern "C" fn yd_record_glmark2_why(line: *const u8, n: usize) -> u32 {
    if n == 0 {
        return record::GLMARK2_WHY_NO_ERROR;
    }
    // SAFETY: the caller's contract.
    record::glmark2_why(unsafe { core::slice::from_raw_parts(line, n) })
}

// ---- the i.MX8MP samples' end (driver/zephyr/lib/board/imx8mp) ------------

const _: () = assert!(size_of::<imx8mp::Hold>() == 36);
const _: () = assert!(offset_of!(imx8mp::Hold, code) == 8);
const _: () = assert!(offset_of!(imx8mp::Hold, reset_max) == 32);

/// The hold before the SoC reset ([`imx8mp::hold_seconds`]): the seconds,
/// and in `*code` the code the hold carries (-1 none).
///
/// # Safety
/// `h` is readable; `code` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hold_seconds(h: *const imx8mp::Hold, code: *mut i32) -> u32 {
    // SAFETY: the caller's contract.
    let (s, c) = imx8mp::hold_seconds(unsafe { &*h });
    // SAFETY: the caller's contract.
    if let Some(code) = unsafe { code.as_mut() } {
        *code = c;
    }
    s
}

// ---- the Raspberry Pi 4's framebuffer (driver/zephyr/lib/board/rpi4) ------

/// The window format for the pixel-order tag's response `code` and `value`
/// (board::rpi4::pixel_order, window_format); `*order` receives the order.
/// 0 when the tag was not answered or the answer is not an order.
///
/// # Safety
/// `order` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn yd_rpi4_window_format(code: u32, value: u32, order: *mut u32) -> u32 {
    let Some(o) = rpi4::pixel_order(code, value) else {
        return 0;
    };
    // SAFETY: the caller's contract.
    if let Some(order) = unsafe { order.as_mut() } {
        *order = o;
    }
    rpi4::window_format(o).unwrap_or(0)
}

/// 1 when a `w` x `h` window can be a view into the framebuffer
/// (board::rpi4::window_is_view), else 0.
///
/// # Safety
/// `fb` is readable.
#[no_mangle]
pub unsafe extern "C" fn yd_rpi4_window_is_view(fb: *const rpi4::Fb, w: u32, h: u32) -> i32 {
    // SAFETY: the caller's contract.
    rpi4::window_is_view(unsafe { &*fb }, w, h) as i32
}

/// 1 and `*row` when the window is a part of the view and board_show() points
/// the screen at that row (board::rpi4::pan_row), 0 when it copies.
///
/// # Safety
/// `fb` is readable, `row` writable.
#[no_mangle]
pub unsafe extern "C" fn yd_rpi4_pan_row(
    fb: *const rpi4::Fb,
    win_phys: u64,
    w: u32,
    h: u32,
    stride: u32,
    row: *mut u32,
) -> i32 {
    // SAFETY: the caller's contract.
    match rpi4::pan_row(unsafe { &*fb }, win_phys, w, h, stride) {
        Some(r) => {
            // SAFETY: the caller's contract.
            unsafe { *row = r };
            1
        }
        None => 0,
    }
}

const _: () = assert!(size_of::<rpi4::Fb>() == 24);
const _: () = assert!(offset_of!(rpi4::Fb, width) == 8);
const _: () = assert!(offset_of!(rpi4::Fb, rows) == 20);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::record::{kind, WHERE_IMAGE};

    #[test]
    fn entry_points_follow_the_rules() {
        let mut w = Writer::new();
        let mut out = [0u32; 4];
        unsafe {
            assert_eq!(yd_record_progress(&mut w, WHERE_IMAGE, 5, &mut out), 1);
            assert_eq!(out, [0x594B0501, 0xA0, 5, 1]);
            assert_eq!(yd_record_breadcrumb(&mut w, kind::STAGE as u32, 0x40, 0, 6, 0, &mut out), 1);
            assert_eq!(out[0], 0x594B0E40);
            assert_eq!(yd_record_result(&mut w, kind::APP as u32, 1, 0, 0, &mut out), 1);
            assert_eq!(out[0], 0x594B0202);
            out = [0; 4];
            assert_eq!(yd_record_progress(&mut w, 0x201, 7, &mut out), 0);
            assert_eq!(out, [0; 4], "a dropped breadcrumb leaves out alone");
            assert_eq!(yd_record_result_tag(&mut w, kind::PROBES as u32, 2, 1, 2, 3, &mut out), 1);
            assert_eq!(out, [0x594B0602, 1, 2, 3]);
        }
    }

    #[test]
    fn glmark2_entry_points_are_the_record_functions() {
        let o = Glmark2Outcome {
            score: 39,
            flags: 0,
            status: record::STATUS_FAIL,
            ran: 30,
            failed: 1,
            unsupported: 2,
            failed_entry: 22,
            why: record::GLMARK2_WHY_FRAGMENT_SHADER,
            unsupported_entries: [17, 19],
            w: 800,
            h: 600,
        };
        let mut out = [0u32; 3];
        let line = b"Error: Failed to link program created from files a and b";
        unsafe {
            yd_record_glmark2_words(&o, &mut out);
            assert_eq!(yd_record_glmark2_why(line.as_ptr(), line.len()), record::GLMARK2_WHY_LINK);
            assert_eq!(yd_record_glmark2_why(core::ptr::null(), 0), record::GLMARK2_WHY_NO_ERROR);
        }
        assert_eq!(out, record::glmark2_words(&o));
    }

    #[test]
    fn the_hold_entry_point_is_hold_seconds() {
        let h = imx8mp::Hold {
            recorded: 1,
            failed: 0,
            code: -1,
            gpu_resets: 2,
            pass_s: 60,
            fail_s: 40,
            step_s: 6,
            reset_base: 56,
            reset_max: 8,
        };
        let mut code = 0;
        // SAFETY: a live struct and a writable int.
        assert_eq!(unsafe { yd_imx8mp_hold_seconds(&h, &mut code) }, 70 + 6 * 57);
        assert_eq!(code, 57);
        // SAFETY: as above; a null code is allowed.
        assert_eq!(unsafe { yd_imx8mp_hold_seconds(&imx8mp::Hold { gpu_resets: 0, ..h }, core::ptr::null_mut()) }, 0);
    }
}
