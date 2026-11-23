// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The record writer for the board layer's C sinks
//! (driver/zephyr/lib/board/common/record.c): the rules of
//! [`crate::board::record::Writer`] behind `yd_record_*`. The C side keeps
//! the writer (`struct yd_record_writer`, include/yakogl_drivers.h), calls
//! one of these, and stores `out` in its sink when the call returns 1.

use crate::board::record::{self, Display, Glmark2Outcome, GmScene, Writer};
use crate::board::record_screen::{self, Ring};
use crate::board::{glmark2_list, imx8mp, rpi4, rpi_fb};
use crate::errno::EINVAL;
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

const _: () = assert!(size_of::<Glmark2Outcome>() == 56);
const _: () = assert!(offset_of!(Glmark2Outcome, failed_entry) == 24);
const _: () = assert!(offset_of!(Glmark2Outcome, unsupported_entries) == 32);
const _: () = assert!(offset_of!(Glmark2Outcome, h) == 44);
const _: () = assert!(offset_of!(Glmark2Outcome, gl_errors) == 48);
const _: () = assert!(offset_of!(Glmark2Outcome, incomplete_draws) == 52);

/// kind GLMARK2's w1..w3 for a run's outcome ([`record::glmark2_words`]).
///
/// # Safety
/// `o` is readable and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn yd_record_glmark2_words(o: *const Glmark2Outcome, out: *mut [u32; 3]) {
    // SAFETY: the caller's contract.
    unsafe { *out = record::glmark2_words(&*o) };
}

const _: () = assert!(size_of::<GmScene>() == 60);
const _: () = assert!(offset_of!(GmScene, probe) == 8);
const _: () = assert!(offset_of!(GmScene, mean) == 36);
const _: () = assert!(offset_of!(GmScene, w) == 52);

/// kind GM_PAGE's w1..w3 for the page `tag` names, from `n` scenes
/// ([`record::gm_page_words`]): 0, or -22 (EINVAL) when the tag names no page
/// or the scenes do not fit it (`out` is then not written).
///
/// # Safety
/// `s` is readable for `n` scenes (or `n` is 0) and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn yd_record_gm_page_words(tag: u32, s: *const GmScene, n: u32, out: *mut [u32; 3]) -> i32 {
    let scenes = if n == 0 {
        &[][..]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(s, n as usize) }
    };
    match record::gm_page_words(tag, scenes) {
        Some(w) => {
            // SAFETY: the caller's contract.
            unsafe { *out = w };
            0
        }
        None => -22,
    }
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

// ---- the glmark2 list at a run's end (board::glmark2_list) ------------------

/// One entry of the list as the C side keeps it: `struct yd_gm_list_scene`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GmListScene {
    pub info: *const u8,
    pub info_len: usize,
    pub error: *const u8,
    pub error_len: usize,
    pub entry: u32,
    pub result: u32,
    pub fps: u32,
    pub why: u32,
}

const _: () = assert!(size_of::<GmListScene>() == 48);
const _: () = assert!(offset_of!(GmListScene, error) == 16);
const _: () = assert!(offset_of!(GmListScene, entry) == 32);
const _: () = assert!(offset_of!(GmListScene, why) == 44);

/// The longest list [`yd_gm_list_line`] takes: every entry is copied to the
/// stack, and the screen shows fewer lines than this.
pub const GM_LIST_MAX: usize = 128;

/// Line `i` of the `n` entries at `list` ([`glmark2_list::line`]) into `out`
/// (`cap` bytes, no newline, no NUL): its length, at most
/// [`glmark2_list::LINE_MAX`]; 0 when `i` is not below `n`; -EINVAL for a null
/// pointer or more than [`GM_LIST_MAX`] entries.
///
/// # Safety
/// `list` is readable for `n` entries, each `info` and `error` for its length
/// (or null with length 0), and `out` writable for `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn yd_gm_list_line(
    list: *const GmListScene,
    n: usize,
    i: usize,
    out: *mut u8,
    cap: usize,
) -> i32 {
    if list.is_null() || out.is_null() || n > GM_LIST_MAX {
        return -EINVAL;
    }
    let bytes = |p: *const u8, len: usize| -> Option<&[u8]> {
        match (p.is_null(), len) {
            (_, 0) => Some(&[]),
            (true, _) => None,
            // SAFETY: the caller's contract.
            (false, _) => Some(unsafe { core::slice::from_raw_parts(p, len) }),
        }
    };
    let empty = glmark2_list::Scene { info: &[], error: &[], entry: 0, result: 0, fps: 0, why: 0 };
    let mut scenes = [empty; GM_LIST_MAX];
    for (k, s) in scenes.iter_mut().enumerate().take(n) {
        // SAFETY: the caller's contract.
        let c = unsafe { *list.add(k) };
        let (Some(info), Some(error)) = (bytes(c.info, c.info_len), bytes(c.error, c.error_len)) else {
            return -EINVAL;
        };
        *s = glmark2_list::Scene { info, error, entry: c.entry, result: c.result, fps: c.fps, why: c.why };
    }
    // SAFETY: the caller's contract.
    let out = unsafe { core::slice::from_raw_parts_mut(out, cap) };
    glmark2_list::line(&scenes[..n], i, out) as i32
}

// ---- kind DISPLAY (driver/zephyr/lib/board/imx8mp, board_finish_record) ----

const _: () = assert!(size_of::<Display>() == 32);
const _: () = assert!(offset_of!(Display, irq_now) == 12);
const _: () = assert!(offset_of!(Display, pitch) == 28);

/// kind DISPLAY's w1..w3 for the display chain's state ([`record::display_words`]).
///
/// # Safety
/// `d` is readable and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn yd_record_display_words(d: *const Display, out: *mut [u32; 3]) {
    // SAFETY: the caller's contract.
    unsafe { *out = record::display_words(&*d) };
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

/// The ARM physical address for the mailbox's framebuffer `bus` address on
/// `board` (4 or 5; board::rpi_fb::fb_phys): 0 for no framebuffer (a zero
/// address, an unknown board).
#[no_mangle]
pub extern "C" fn yd_rpi_fb_phys(board: u32, bus: u32) -> u64 {
    rpi_fb::Board::from_u32(board).and_then(|b| rpi_fb::fb_phys(b, bus)).unwrap_or(0)
}

/// 1 when a FRAMEBUFFER_ALLOCATE answer (`pitch`, `size`) holds `rows` rows of
/// `width` 32 bpp pixels (board::rpi_fb::allocate_ok), else 0.
#[no_mangle]
pub extern "C" fn yd_rpi_allocate_ok(width: u32, rows: u32, pitch: u32, size: u32) -> i32 {
    rpi_fb::allocate_ok(width, rows, pitch, size) as i32
}

/// 1 and `*fb` when the mailbox answer is a usable framebuffer on `board`
/// (board::rpi_fb::fb_from_answer), 0 otherwise.
///
/// # Safety
/// `fb` is writable.
#[no_mangle]
pub unsafe extern "C" fn yd_rpi_fb_from_answer(
    board: u32,
    bus: u32,
    width: u32,
    height: u32,
    rows: u32,
    pitch: u32,
    size: u32,
    fb: *mut rpi4::Fb,
) -> i32 {
    let Some(b) = rpi_fb::Board::from_u32(board) else {
        return 0;
    };
    match rpi_fb::fb_from_answer(b, bus, width, height, rows, pitch, size) {
        Some(f) => {
            // SAFETY: the caller's contract.
            unsafe { *fb = f };
            1
        }
        None => 0,
    }
}

/// 1, `*fb` and the window format in `*format` when the plane the firmware
/// scans out can be adopted (board::rpi_fb::adopt_scanout; `fmt` is
/// YD_RPI_SCANOUT_FMT_*), 0 otherwise.
///
/// # Safety
/// `fb` and `format` are writable.
#[no_mangle]
pub unsafe extern "C" fn yd_rpi_adopt_scanout(
    phys: u64,
    pitch: u32,
    width: u32,
    height: u32,
    fmt: u32,
    fb: *mut rpi4::Fb,
    format: *mut u32,
) -> i32 {
    match rpi_fb::adopt_scanout(phys, pitch, width, height, fmt) {
        Some((f, w)) => {
            // SAFETY: the caller's contract.
            unsafe {
                *fb = f;
                *format = w;
            }
            1
        }
        None => 0,
    }
}

const _: () = assert!(size_of::<rpi4::Fb>() == 24);
const _: () = assert!(offset_of!(rpi4::Fb, width) == 8);
const _: () = assert!(offset_of!(rpi4::Fb, rows) == 20);

/// One console character into the record screen's ring
/// ([`record_screen::Ring::put`]; the low byte of `c`). A null `r` is
/// ignored.
///
/// # Safety
/// `r` is null or points to a ring the caller owns, with no other call on
/// it running.
#[no_mangle]
pub unsafe extern "C" fn yd_record_screen_put(r: *mut Ring, c: u32) {
    // SAFETY: the caller's contract.
    if let Some(r) = unsafe { r.as_mut() } {
        r.put(c as u8);
    }
}

/// Draw the record screen ([`record_screen::Ring::draw`]) over the `width` x
/// `height` XRGB8888 picture at `px` (rows `stride_bytes` apart, top row
/// first) with the 8x16 glyphs of printable ASCII at `font` and the
/// `head_len` characters at `head` as the first row. Returns the console
/// lines drawn; 0 with nothing written for a picture of fewer than three
/// text rows of one character; -EINVAL with nothing written for a null or
/// unaligned pointer or a stride that is no whole pixels.
///
/// # Safety
/// `r` is a ring no `yd_record_screen_put` runs on during the call; `px` is
/// valid for `stride_bytes * (height - 1) + width * 4` bytes of writes,
/// `font` for [`record_screen::FONT_GLYPHS`] glyphs and `head` for
/// `head_len` bytes of reads.
#[no_mangle]
pub unsafe extern "C" fn yd_record_screen_draw(
    r: *const Ring,
    px: *mut u32,
    width: u32,
    height: u32,
    stride_bytes: u32,
    font: *const record_screen::Font,
    head: *const u8,
    head_len: usize,
) -> i32 {
    if r.is_null() || px.is_null() || !px.is_aligned() || font.is_null() || (head.is_null() && head_len != 0) {
        return -EINVAL;
    }
    if !stride_bytes.is_multiple_of(4) {
        return -EINVAL;
    }
    let (stride, width, height) = (stride_bytes as usize / 4, width as usize, height as usize);
    if height == 0 || stride < width {
        return 0;
    }
    let Some(len) = stride.checked_mul(height - 1).and_then(|n| n.checked_add(width)) else { return 0 };
    // SAFETY: the caller's contract; `len` pixels are the bytes it names.
    let (r, px, font, head) = unsafe {
        let head = if head_len == 0 { &[][..] } else { core::slice::from_raw_parts(head, head_len) };
        (&*r, core::slice::from_raw_parts_mut(px, len), &*font, head)
    };
    r.draw(px, stride, width, height, font, head) as i32
}

const _: () = assert!(size_of::<Ring>() == 7794);
const _: () = assert!(size_of::<record_screen::Font>() == 95 * 16);

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

    /// The record screen's entry points: the ring's lines reach the pixels,
    /// and what the call cannot draw into is refused before any write.
    #[test]
    fn record_screen_entry_points_draw_or_refuse() {
        use record_screen::{BG, FG, FONT_H};
        let mut font = [[0u8; FONT_H]; 95];
        font[(b'a' - 0x20) as usize][0] = 0x80; // 'a': the cell's top left pixel
        let mut r = Ring::new();
        let (w, h, stride) = (16u32, 3 * FONT_H as u32, 20u32);
        let mut px = vec![1u32; (stride * (h - 1) + w) as usize];
        let n = px.len();
        unsafe {
            yd_record_screen_put(core::ptr::null_mut(), b'a' as u32);
            for c in *b" a\n" {
                yd_record_screen_put(&mut r, 0x100 | c as u32); // the low byte counts
            }
            let draw = |px: *mut u32, stride_bytes, font: *const record_screen::Font, head: *const u8, head_len| {
                yd_record_screen_draw(&r, px, w, h, stride_bytes, font, head, head_len)
            };
            let p = px.as_mut_ptr();
            assert_eq!(draw(core::ptr::null_mut(), stride * 4, &font, b"a".as_ptr(), 1), -EINVAL);
            assert_eq!(draw(p.cast::<u8>().add(1).cast(), stride * 4, &font, b"a".as_ptr(), 1), -EINVAL);
            assert_eq!(draw(p, stride * 4 + 2, &font, b"a".as_ptr(), 1), -EINVAL);
            assert_eq!(draw(p, stride * 4, core::ptr::null(), b"a".as_ptr(), 1), -EINVAL);
            assert_eq!(draw(p, stride * 4, &font, core::ptr::null(), 1), -EINVAL);
            assert_eq!(yd_record_screen_draw(core::ptr::null(), p, w, h, stride * 4, &font, b"a".as_ptr(), 1), -EINVAL);
            assert_eq!(draw(p, (w - 1) * 4, &font, b"a".as_ptr(), 1), 0, "rows closer than the width");
            assert_eq!(yd_record_screen_draw(&r, p, w, 0, stride * 4, &font, b"a".as_ptr(), 1), 0);
            assert!(px.iter().all(|&p| p == 1), "a refused call wrote");
            // a null head of no characters is an empty first row
            assert_eq!(draw(p, stride * 4, &font, core::ptr::null(), 0), 1);
        }
        assert_eq!(px.len(), n);
        let at = |x: u32, y: u32| px[(y * stride + x) as usize];
        // row 1 is the line " a": the second cell's top left pixel
        assert_eq!((at(8, FONT_H as u32), at(0, FONT_H as u32), at(0, 0)), (FG, BG, BG));
        assert_eq!(at(w, 0), 1, "a pixel between the rows' ends is not the picture's");
    }

    #[test]
    fn glmark2_entry_points_are_the_record_functions() {
        let o = Glmark2Outcome {
            score: 39,
            flags: record::GLMARK2_GL,
            status: record::STATUS_FAIL,
            ran: 30,
            failed: 1,
            unsupported: 2,
            failed_entry: 22,
            why: record::GLMARK2_WHY_FRAGMENT_SHADER,
            unsupported_entries: [17, 19],
            w: 800,
            h: 600,
            gl_errors: 11,
            incomplete_draws: 3,
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

    /// The list's entry point: the lines of `glmark2_list::line`, and what it
    /// cannot read is refused.
    #[test]
    fn gm_list_entry_point_is_the_list_function() {
        let (a, b, e) = (b"[build] <use-vbo=false>", b"[build] <use-vbo=true>", b"Error: Could not open 3ds file 'x'");
        let s = |info: &[u8], error: &[u8], entry, result| GmListScene {
            info: info.as_ptr(),
            info_len: info.len(),
            error: error.as_ptr(),
            error_len: error.len(),
            entry,
            result,
            fps: 61,
            why: record::GLMARK2_WHY_FILE,
        };
        let mut list = [s(a, b"", 0, glmark2_list::RAN), s(b, e, 1, glmark2_list::FAILED)];
        let mut out = [0u8; 160];
        let line = |list: &[GmListScene], i, out: &mut [u8]| unsafe {
            yd_gm_list_line(list.as_ptr(), list.len(), i, out.as_mut_ptr(), out.len())
        };
        let n = line(&list, 1, &mut out);
        assert_eq!(
            &out[..n as usize],
            b"glmark2-list: 1 build:use-vbo=true  failed, a file: Could not open 3ds file 'x'"
        );
        assert_eq!(line(&list, 2, &mut out), 0);
        list[0].error = core::ptr::null();
        assert_eq!(line(&list, 0, &mut out), 46, "a null error of no bytes is none");
        list[0].error_len = 1;
        assert_eq!(line(&list, 0, &mut out), -EINVAL);
        let many = [list[1]; GM_LIST_MAX + 1];
        assert_eq!(line(&many, 0, &mut out), -EINVAL);
        assert_eq!(unsafe { yd_gm_list_line(core::ptr::null(), 0, 0, out.as_mut_ptr(), 1) }, -EINVAL);
        // the C header's numbers
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let def = |name: &str| -> usize {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            l.split_whitespace().nth(2).expect(name).parse().unwrap_or_else(|_| panic!("{name}"))
        };
        for (name, v) in [
            ("YD_GM_LIST_RAN", glmark2_list::RAN as usize),
            ("YD_GM_LIST_FAILED", glmark2_list::FAILED as usize),
            ("YD_GM_LIST_UNSUPPORTED", glmark2_list::UNSUPPORTED as usize),
            ("YD_GM_LIST_NO_END", glmark2_list::NO_END as usize),
            ("YD_GM_LIST_LINE_MAX", glmark2_list::LINE_MAX),
            ("YD_GM_LIST_MAX", GM_LIST_MAX),
        ] {
            assert_eq!(def(name), v, "{name}");
        }
    }

    #[test]
    fn the_gm_page_entry_point_is_the_record_function() {
        let s = GmScene { fps: 61, frames: 613, w: 800, h: 600, ..GmScene::default() };
        let mut out = [7u32; 3];
        let p1 = record::GM_SEL_P1 << record::GM_SEL_T_SHIFT | 4;
        assert_eq!(unsafe { yd_record_gm_page_words(p1, &s, 1, &mut out) }, 0);
        assert_eq!(Some(out), record::gm_page_words(p1, &[s]));
        // no scene for a P page, a RATE tag: refused, out untouched
        let mut out = [7u32; 3];
        assert_eq!(unsafe { yd_record_gm_page_words(p1, core::ptr::null(), 0, &mut out) }, -22);
        let rate = record::GM_SEL_RATE << record::GM_SEL_T_SHIFT;
        assert_eq!(unsafe { yd_record_gm_page_words(rate, &s, 1, &mut out) }, -22);
        assert_eq!(out, [7; 3]);
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
