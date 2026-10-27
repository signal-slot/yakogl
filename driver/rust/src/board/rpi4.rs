// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Raspberry Pi 4 board layer's decisions about its framebuffer
//! (driver/zephyr/lib/board/rpi4/board_rpi4.c, through `ffi::board`): what
//! the firmware's pixel-order answer means for the window's format, whether
//! a window can be a view into the framebuffer (CONFIG_YAKOGL_BOARD_SCANOUT_
//! VIEW in yakogl_board.h), and the row a part of the view starts at (the
//! pan). Pure
//! functions, so the host tests below cover them; the mailbox calls stay in
//! C (rpi_mbox.c).

/// `struct gles_native_window.format` values (include/EGL/eglplatform.h).
pub const FORMAT_RGBA8888: u32 = 1;
pub const FORMAT_XRGB8888: u32 = 2;

/// The mailbox's pixel orders (tags 0x00040006 GET / 0x00048006 SET_PIXEL_ORDER,
/// raspberrypi-firmware.h: 0 BGR, 1 RGB).
pub const PIXEL_ORDER_BGR: u32 = 0;
pub const PIXEL_ORDER_RGB: u32 = 1;

/// Bit 31 of a tag's request/response code: the firmware answered the tag.
pub const TAG_ANSWERED: u32 = 0x8000_0000;

/// The pixel order the firmware answered, from a pixel-order tag's response
/// code word and its value: `None` when the tag was not answered, the answer
/// is not one word (bits 30..0 of the code are the value length in bytes),
/// or the value is neither order.
pub fn pixel_order(code: u32, value: u32) -> Option<u32> {
    if code & TAG_ANSWERED == 0 || code & !TAG_ANSWERED != 4 {
        return None;
    }
    match value {
        PIXEL_ORDER_BGR | PIXEL_ORDER_RGB => Some(value),
        _ => None,
    }
}

/// The window's format for a 32 bpp framebuffer of `order`: the bytes the
/// display scans, read as a little-endian word. RGB order puts R in the
/// pixel's first byte (R, G, B, X in memory: RGBA8888, as v3d_gl_slint and
/// v3d_gl_cluster drew into this framebuffer); BGR puts B first (B, G, R, X:
/// XRGB8888, 0xXXRRGGBB). Whether the panel shows it so is a camera's
/// question, not this function's.
pub fn window_format(order: u32) -> Option<u32> {
    match order {
        PIXEL_ORDER_RGB => Some(FORMAT_RGBA8888),
        PIXEL_ORDER_BGR => Some(FORMAT_XRGB8888),
        _ => None,
    }
}

/// The framebuffer as the board layer holds it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Fb {
    pub phys: u64,
    pub width: u32,
    /// The screen's height (the rows on screen at once).
    pub height: u32,
    /// Bytes per row.
    pub pitch: u32,
    /// Rows in memory: the virtual height.
    pub rows: u32,
}

/// Whether a `w` x `h` window can be a view into the framebuffer: at its
/// (0,0) with its pitch, at most its width and at most its rows in memory
/// (two screens hold a double buffer), and the pitch an even number of
/// whole pixels (the GPUs write pixel pairs; YakoGL takes such a pitch for
/// a window, src/gl/egl.rs native_window_pitch). Then board_window() hands
/// out the view and board_show() copies nothing; any other window is a
/// buffer of its own, copied.
pub fn window_is_view(fb: &Fb, w: u32, h: u32) -> bool {
    w != 0 && h != 0 && w <= fb.width && h <= fb.rows && fb.pitch.is_multiple_of(8) && fb.pitch as u64 >= w as u64 * 4
}

/// The row the screen starts at to show a part of the view: at most the
/// screen's width and height, at the framebuffer's pitch, at a row boundary,
/// and the screen's rows from there inside the rows in memory (the screen
/// always scans a whole screen). `None`: not such a part (the copy).
pub fn pan_row(fb: &Fb, win_phys: u64, w: u32, h: u32, stride: u32) -> Option<u32> {
    if w == 0 || h == 0 || w > fb.width || h > fb.height || stride != fb.pitch || fb.pitch == 0 || win_phys < fb.phys {
        return None;
    }
    let off = win_phys - fb.phys;
    if !off.is_multiple_of(fb.pitch as u64) {
        return None;
    }
    let row = off / fb.pitch as u64;
    (row + fb.height as u64 <= fb.rows as u64).then_some(row as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cluster's framebuffer: 1920x720, two screens (the grant
    /// v3d_fb_flip measured), at an ARM address of the Pi 4's.
    const FB: Fb = Fb { phys: 0x3c10_0000, width: 1920, height: 720, pitch: 1920 * 4, rows: 1440 };

    #[test]
    fn pixel_order_tag() {
        assert_eq!(pixel_order(TAG_ANSWERED | 4, 1), Some(PIXEL_ORDER_RGB));
        assert_eq!(pixel_order(TAG_ANSWERED | 4, 0), Some(PIXEL_ORDER_BGR));
        assert_eq!(pixel_order(4, 1), None, "not answered");
        assert_eq!(pixel_order(0, 0), None, "the request as sent, unanswered");
        assert_eq!(pixel_order(TAG_ANSWERED | 8, 1), None, "two words is not this tag's answer");
        assert_eq!(pixel_order(TAG_ANSWERED | 4, 2), None, "no such order");
        assert_eq!(window_format(PIXEL_ORDER_RGB), Some(FORMAT_RGBA8888));
        assert_eq!(window_format(PIXEL_ORDER_BGR), Some(FORMAT_XRGB8888));
        assert_eq!(window_format(7), None);
    }

    /// The cluster's 2 x 720 window and slint's 1024x608 window are views;
    /// a window wider than the screen or taller than the rows, and a pitch
    /// that is not whole pixel pairs, are not.
    #[test]
    fn which_windows_are_views() {
        assert!(window_is_view(&FB, 1920, 1440));
        assert!(window_is_view(&FB, 1920, 720));
        assert!(window_is_view(&FB, 1024, 608), "slint's window: a view at the screen's pitch");
        assert!(!window_is_view(&FB, 1920, 1441), "taller than the rows in memory");
        assert!(!window_is_view(&FB, 1921, 720), "wider than the screen");
        assert!(!window_is_view(&FB, 0, 720));
        assert!(!window_is_view(&FB, 1920, 0));
        let one = Fb { phys: 0x3c10_0000, width: 1280, height: 720, pitch: 1280 * 4, rows: 720 };
        assert!(window_is_view(&one, 1024, 608), "one screen: a smaller window");
        assert!(!window_is_view(&one, 1280, 1440), "one screen holds no double buffer");
        let padded = Fb { pitch: 1920 * 4 + 64, ..FB };
        assert!(window_is_view(&padded, 1920, 1440), "a wider pitch is a view's pitch");
        let odd = Fb { pitch: 1920 * 4 + 4, ..FB };
        assert!(!window_is_view(&odd, 1920, 1440), "not an even number of whole pixels");
    }

    /// The halves pan to rows 0 and 720, a smaller view at (0,0) to row 0;
    /// anything else is a copy.
    #[test]
    fn pan_rows() {
        let half = FB.pitch as u64 * 720;
        assert_eq!(pan_row(&FB, FB.phys, 1920, 720, FB.pitch), Some(0));
        assert_eq!(pan_row(&FB, FB.phys + half, 1920, 720, FB.pitch), Some(720));
        assert_eq!(pan_row(&FB, FB.phys + FB.pitch as u64 * 3, 1920, 720, FB.pitch), Some(3));
        assert_eq!(pan_row(&FB, FB.phys, 1024, 600, FB.pitch), Some(0), "a smaller view");
        assert_eq!(pan_row(&FB, FB.phys + half + FB.pitch as u64, 1920, 720, FB.pitch), None, "past the rows");
        assert_eq!(
            pan_row(&FB, FB.phys + half + FB.pitch as u64, 1920, 600, FB.pitch),
            None,
            "the screen scans 720 rows"
        );
        assert_eq!(pan_row(&FB, FB.phys + 4, 1920, 720, FB.pitch), None, "not at a row boundary");
        assert_eq!(pan_row(&FB, FB.phys - FB.pitch as u64, 1920, 720, FB.pitch), None, "before the framebuffer");
        assert_eq!(pan_row(&FB, FB.phys, 1920, 721, FB.pitch), None, "taller than the screen");
        assert_eq!(pan_row(&FB, FB.phys, 1024, 600, 1024 * 4), None, "a packed buffer, not the view");
        assert_eq!(pan_row(&FB, 0x1000_0000, 1920, 720, FB.pitch), None, "a static buffer");
        let one = Fb { rows: 720, ..FB };
        assert_eq!(pan_row(&one, one.phys, 1920, 720, one.pitch), Some(0));
        assert_eq!(pan_row(&one, one.phys + one.pitch as u64, 1920, 600, one.pitch), None, "one screen: row 0 only");
    }
}
