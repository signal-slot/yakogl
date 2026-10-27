// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The one check of the Slint sample (examples/slint), the same
//! on the Raspberry Pi 4 (V3D), the Verdin AM62P (PowerVR) and the Verdin
//! iMX8M Plus (GC7000) (driver/docs/one-method.md, section 5, step 1).
//!
//! Moved here from `v3d_gl_slint/src/main.c` (the probe table at :353-357,
//! the comparison at :340-371 and :547-590), which had the most complete
//! check of the four Slint samples (one-method.md section 1.4). What it
//! checks in the cluster's first frame (`demo_vehicle` at t = 0, two
//! `slint_draw_frame` calls, one swap):
//!
//! - the five points of [`REFERENCE`], each channel within [`TOLERANCE`] of
//!   the AM62P's reading;
//! - no pixel of the window keeps the CPU's [`SENTINEL`] (the whole window
//!   was drawn), and every pixel's alpha byte is 255 (the cluster is opaque);
//! - the [`GUARD_ROWS`] rows after the window's memory still hold what they
//!   held before the frame (the GPU wrote nothing past the window): the
//!   sample's snapshot of them ([`Frame::with_guard`]), or [`GUARD`] when no
//!   snapshot is given (the host tests' synthetic frames). The sample never
//!   paints them: in a view (CONFIG_YAKOGL_BOARD_SCANOUT_VIEW) they are on
//!   the screen;
//! - glReadPixels agrees with memory at the five points.
//!
//! The window's CRC-32 (rows top-down as the eye sees them) is the
//! FINGERPRINT record. It is not a verdict: whether three rasterisers give
//! bit-equal anti-aliasing is unmeasured.
//!
//! `rust_slint`'s expectation for "track", (20,26,34)
//! (`rust_slint/src/lib.rs:396`), is not carried over: it was never measured,
//! and the AM62P reads (13,17,24) there (one-method.md section 1.4).
#![cfg_attr(not(test), no_std)]

/// The result record, the one definition (the board layer's), compiled into
/// this crate as its header says to (no second copy).
#[path = "../../../../driver/rust/src/board/record.rs"]
pub mod record;

/// The window, the AM62P panel's size, which fits all three panels
/// (1024x600, 1280x800, 1920x720).
pub const WIDTH: u32 = 1024;
pub const HEIGHT: u32 = 600;
/// Rows after the window's memory that the GPU must not touch.
pub const GUARD_ROWS: u32 = 8;
/// Written by the CPU into every window pixel before the first frame: R 1
/// (B in XRGB8888), alpha 0. Near black, so a view (the window on the screen
/// itself) shows nothing a user would see before the first frame lands; and
/// an opaque drawn pixel never has alpha 0, so a pixel that keeps it was not
/// drawn.
pub const SENTINEL: u32 = 0x0000_0001;
/// What the guard rows hold in the host tests' synthetic frames; on a board
/// they hold whatever they held before the frame (the sample's snapshot).
pub const GUARD: u32 = 0xff00_00ff;
/// Per channel, 8-bit.
pub const TOLERANCE: u8 = 1;
/// The frames that make up the checked window (FINGERPRINT's w2).
pub const FRAMES: u32 = 2;

/// `struct gles_native_window.format` values (include/EGL/eglplatform.h).
pub const FORMAT_RGBA8888: u32 = 1;
pub const FORMAT_XRGB8888: u32 = 2;
pub const FORMAT_BOTTOM_UP: u32 = 0x8000_0000;

/// A reference point: UI coordinates (y down), and the expected R, G, B.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub what: &'static str,
    pub x: u32,
    pub y: u32,
    pub want: [u8; 3],
}

/// The reference table, the one place it is kept. The points are
/// `gl_slint`'s `readback_first_frame()` (static elements of the design:
/// background, flat dial faces, the left dial's track), and the values are
/// the AM62P's own reading of them through YakoGL on the PowerVR
/// (tools/cts/RESULTS.md, "gl_slint on Zephyr (AM62P) against a host
/// reference render", 2026-09-23: all five within 1 of the host render).
pub const REFERENCE: [Point; 5] = [
    Point { what: "bg-top-left", x: 40, y: 20, want: [11, 16, 23] },
    Point { what: "face-left", x: 214, y: 330, want: [9, 12, 18] },
    Point { what: "face-right", x: 810, y: 330, want: [8, 11, 17] },
    Point { what: "bg-bottom", x: 300, y: 590, want: [4, 6, 10] },
    Point { what: "track", x: 47, y: 330, want: [13, 17, 24] },
];

/// Why a frame could not be checked at all (not a verdict on the frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A zero size, or one the reference points do not fit in.
    Size,
    /// Not RGBA8888 or XRGB8888 (with or without the bottom-up flag).
    Format,
    /// Fewer pixels than the window and its guard rows need at the stride.
    Short,
    /// A row stride shorter than the width.
    Stride,
}

impl FrameError {
    pub fn what(self) -> &'static str {
        match self {
            FrameError::Size => "the window is smaller than the reference points need",
            FrameError::Format => "the window format is neither RGBA8888 nor XRGB8888",
            FrameError::Short => "the window memory is shorter than the window and its guard rows",
            FrameError::Stride => "the window's row stride is shorter than its width",
        }
    }
}

/// The window's memory as the check sees it.
pub struct Frame<'a> {
    pixels: &'a [u32],
    w: u32,
    h: u32,
    /// Words from one row's start to the next: `w`, or wider for a window
    /// that is a view into a wider scan-out buffer
    /// (yakogl_board.h, CONFIG_YAKOGL_BOARD_SCANOUT_VIEW).
    stride: u32,
    guard_rows: u32,
    /// The guard rows as they were before the frame (`guard_rows` rows of
    /// `w` words), or None: [`GUARD`] everywhere.
    guard_want: Option<&'a [u32]>,
    xrgb: bool,
    bottom_up: bool,
}

impl<'a> Frame<'a> {
    /// `pixels` holds `(h + guard_rows) * w` words, rows tightly packed, the
    /// guard rows after the window's last row in memory. `format` is the
    /// native window's format word as the board gave it.
    pub fn new(pixels: &'a [u32], w: u32, h: u32, guard_rows: u32, format: u32) -> Result<Self, FrameError> {
        Self::with_stride(pixels, w, h, w, guard_rows, format)
    }

    /// As [`Frame::new`], with rows `stride` words apart (a view into a
    /// wider buffer): `pixels` holds `(h + guard_rows - 1) * stride + w`
    /// words; only the first `w` of each row are the window's (or its guard
    /// row's), the rest belong to whatever the buffer is and are never read.
    pub fn with_stride(
        pixels: &'a [u32],
        w: u32,
        h: u32,
        stride: u32,
        guard_rows: u32,
        format: u32,
    ) -> Result<Self, FrameError> {
        let fits = REFERENCE.iter().all(|p| p.x < w && p.y < h);
        if w == 0 || h == 0 || !fits {
            return Err(FrameError::Size);
        }
        if stride < w {
            return Err(FrameError::Stride);
        }
        let xrgb = match format & !FORMAT_BOTTOM_UP {
            FORMAT_RGBA8888 => false,
            FORMAT_XRGB8888 => true,
            _ => return Err(FrameError::Format),
        };
        let need = (h as usize + guard_rows as usize - 1) * stride as usize + w as usize;
        if pixels.len() < need {
            return Err(FrameError::Short);
        }
        Ok(Frame {
            pixels,
            w,
            h,
            stride,
            guard_rows,
            guard_want: None,
            xrgb,
            bottom_up: format & FORMAT_BOTTOM_UP != 0,
        })
    }

    /// The guard rows are checked against `before` (their `guard_rows * w`
    /// words as the sample read them before the frame) instead of [`GUARD`].
    /// [`FrameError::Short`] when `before` is shorter than that.
    pub fn with_guard(mut self, before: &'a [u32]) -> Result<Self, FrameError> {
        if before.len() < self.guard_rows as usize * self.w as usize {
            return Err(FrameError::Short);
        }
        self.guard_want = Some(before);
        Ok(self)
    }

    /// Memory row `r` (0 = the first in memory; `h..` the guard rows), its
    /// `w` words.
    fn mem_row(&self, r: u32) -> &[u32] {
        let start = r as usize * self.stride as usize;
        &self.pixels[start..start + self.w as usize]
    }

    /// Row `y` as the eye sees it (0 = top). A bottom-up window keeps the
    /// bottom row first in memory.
    fn row(&self, y: u32) -> &[u32] {
        let mem_row = if self.bottom_up { self.h - 1 - y } else { y };
        self.mem_row(mem_row)
    }

    /// A memory word as R, G, B, A (A is the X byte of XRGB8888).
    fn rgba(&self, px: u32) -> [u8; 4] {
        let b = px.to_le_bytes();
        if self.xrgb {
            [b[2], b[1], b[0], b[3]]
        } else {
            b
        }
    }
}

/// Which part of the check failed first, in `v3d_gl_slint`'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Points,
    Undrawn,
    Alpha,
    Guard,
    ReadPixels,
}

impl Failure {
    pub fn what(self) -> &'static str {
        match self {
            Failure::Points => "the AM62P reference points",
            Failure::Undrawn => "part of the window was not drawn",
            Failure::Alpha => "alpha is not 255 everywhere",
            Failure::Guard => "the guard rows after the window changed",
            Failure::ReadPixels => "glReadPixels differs from memory",
        }
    }
}

/// Everything the check measured; each count is over the whole window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Memory at the reference points, R, G, B, A.
    pub points: [[u8; 4]; 5],
    /// glReadPixels at the reference points, R, G, B, A.
    pub via_gl: [[u8; 4]; 5],
    /// The largest channel difference to the reference, per point.
    pub diff: [u8; 5],
    pub points_bad: u32,
    pub reads_bad: u32,
    pub sentinel_left: u32,
    pub alpha_bad: u32,
    pub guard_bad: u32,
    /// CRC-32/IEEE of the window, rows top-down, each row's memory bytes.
    pub crc32: u32,
}

impl Outcome {
    /// The first failure, or None for a pass.
    pub fn verdict(&self) -> Option<Failure> {
        if self.points_bad != 0 {
            Some(Failure::Points)
        } else if self.sentinel_left != 0 {
            Some(Failure::Undrawn)
        } else if self.alpha_bad != 0 {
            Some(Failure::Alpha)
        } else if self.guard_bad != 0 {
            Some(Failure::Guard)
        } else if self.reads_bad != 0 {
            Some(Failure::ReadPixels)
        } else {
            None
        }
    }

    /// PROBES' status (the low byte of w0): the whole check, not only the
    /// points.
    pub fn status(&self) -> u8 {
        let s = if self.verdict().is_none() { record::STATUS_PASS } else { record::STATUS_FAIL };
        s as u8
    }

    /// PROBES' w1..w3: the five points from memory.
    pub fn probes_words(&self) -> [u32; 3] {
        let mut rgb = [[0u8; 3]; 5];
        for (o, p) in rgb.iter_mut().zip(self.points.iter()) {
            o.copy_from_slice(&p[..3]);
        }
        record::probes_words(&rgb)
    }

    /// FINGERPRINT's w1..w3 for a `w` x `h` window.
    pub fn fingerprint_words(&self, w: u32, h: u32) -> [u32; 3] {
        [self.crc32, FRAMES, record::size_word(w, h)]
    }
}

fn max_diff(have: [u8; 4], want: [u8; 3]) -> u8 {
    (0..3).map(|c| have[c].abs_diff(want[c])).max().unwrap_or(0)
}

/// Check `frame`. `read_gl(x, y)` is glReadPixels of one RGBA pixel at GL
/// window coordinates (origin bottom left), with the frame's context
/// current.
pub fn check(frame: &Frame, mut read_gl: impl FnMut(u32, u32) -> [u8; 4]) -> Outcome {
    let mut o = Outcome {
        points: [[0; 4]; 5],
        via_gl: [[0; 4]; 5],
        diff: [0; 5],
        points_bad: 0,
        reads_bad: 0,
        sentinel_left: 0,
        alpha_bad: 0,
        guard_bad: 0,
        crc32: 0,
    };
    let mut bytes = [0u8; 1024];
    for y in 0..frame.h {
        let row = frame.row(y);
        for &px in row {
            o.sentinel_left += (px == SENTINEL) as u32;
            o.alpha_bad += (frame.rgba(px)[3] != 0xff) as u32;
        }
        for chunk in row.chunks(bytes.len() / 4) {
            for (i, px) in chunk.iter().enumerate() {
                bytes[4 * i..4 * i + 4].copy_from_slice(&px.to_le_bytes());
            }
            o.crc32 = record::crc32_ieee_update(o.crc32, &bytes[..4 * chunk.len()]);
        }
    }
    o.guard_bad = (0..frame.guard_rows)
        .map(|g| {
            let row = frame.mem_row(frame.h + g);
            let w = frame.w as usize;
            let bad = match frame.guard_want {
                Some(b) => row.iter().zip(&b[g as usize * w..(g as usize + 1) * w]).filter(|(a, b)| a != b).count(),
                None => row.iter().filter(|&&px| px != GUARD).count(),
            };
            bad as u32
        })
        .sum();
    for (i, p) in REFERENCE.iter().enumerate() {
        let mem = frame.rgba(frame.row(p.y)[p.x as usize]);
        let gl = read_gl(p.x, frame.h - 1 - p.y);
        o.points[i] = mem;
        o.via_gl[i] = gl;
        o.diff[i] = max_diff(mem, p.want);
        o.points_bad += (o.diff[i] > TOLERANCE) as u32;
        o.reads_bad += (gl != mem) as u32;
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = WIDTH;
    const H: u32 = HEIGHT;
    /// An opaque colour no reference point has.
    const FILL: [u8; 3] = [30, 40, 50];

    /// A synthetic drawn frame: FILL everywhere, the reference values at the
    /// reference points (moved by `delta` on the `channel` of point
    /// `which`), guard rows intact.
    fn frame(format: u32, which: usize, channel: usize, delta: i16) -> Vec<u32> {
        let xrgb = format & !FORMAT_BOTTOM_UP == FORMAT_XRGB8888;
        let word = |c: [u8; 3]| {
            if xrgb {
                u32::from_le_bytes([c[2], c[1], c[0], 0xff])
            } else {
                u32::from_le_bytes([c[0], c[1], c[2], 0xff])
            }
        };
        let mut px = vec![word(FILL); (W * H) as usize];
        px.extend(core::iter::repeat_n(GUARD, (W * GUARD_ROWS) as usize));
        for (i, p) in REFERENCE.iter().enumerate() {
            let mut c = p.want;
            if i == which {
                c[channel] = (c[channel] as i16 + delta) as u8;
            }
            let mem_row = if format & FORMAT_BOTTOM_UP != 0 { H - 1 - p.y } else { p.y };
            px[(mem_row * W + p.x) as usize] = word(c);
        }
        px
    }

    /// glReadPixels as a correct GL would answer it for `px`: GL row y is
    /// the eye's row H-1-y.
    fn gl_reader(px: &[u32], format: u32) -> impl FnMut(u32, u32) -> [u8; 4] + '_ {
        move |x, y| {
            let f = Frame::new(px, W, H, GUARD_ROWS, format).unwrap();
            f.rgba(f.row(H - 1 - y)[x as usize])
        }
    }

    fn run(px: &[u32], format: u32) -> Outcome {
        let f = Frame::new(px, W, H, GUARD_ROWS, format).unwrap();
        check(&f, gl_reader(px, format))
    }

    #[test]
    fn reference_values_pass() {
        for format in
            [FORMAT_RGBA8888, FORMAT_XRGB8888, FORMAT_RGBA8888 | FORMAT_BOTTOM_UP, FORMAT_XRGB8888 | FORMAT_BOTTOM_UP]
        {
            let o = run(&frame(format, 0, 0, 0), format);
            assert_eq!(o.verdict(), None, "format {format:#x}: {o:?}");
            assert_eq!(o.diff, [0; 5]);
            assert_eq!(o.status(), record::STATUS_PASS as u8);
            for (have, p) in o.points.iter().zip(REFERENCE.iter()) {
                assert_eq!(have[..3], p.want, "{}", p.what);
            }
        }
    }

    /// The tolerance, on every point and channel, in both directions: a
    /// value 1 away passes, 2 away fails and names the points.
    #[test]
    fn tolerance_is_one_per_channel() {
        let mut cases = 0;
        for which in 0..5 {
            for channel in 0..3 {
                for (delta, pass) in [(1i16, true), (-1, true), (2, false), (-2, false)] {
                    let o = run(&frame(FORMAT_RGBA8888, which, channel, delta), FORMAT_RGBA8888);
                    let want = if pass { None } else { Some(Failure::Points) };
                    assert_eq!(o.verdict(), want, "point {which} channel {channel} delta {delta}");
                    assert_eq!(o.diff[which], delta.unsigned_abs() as u8);
                    assert_eq!(o.points_bad, (!pass) as u32);
                    let status = if pass { record::STATUS_PASS } else { record::STATUS_FAIL };
                    assert_eq!(o.status(), status as u8);
                    cases += 1;
                }
            }
        }
        // 5 points x 3 channels x 4 deltas (every reference channel is >= 4,
        // so -2 never wraps).
        assert!(REFERENCE.iter().all(|p| p.want.iter().all(|&c| c >= 2)));
        assert_eq!(cases, 60);
    }

    #[test]
    fn sentinel_alpha_guard_and_readpixels_each_fail() {
        let f = FORMAT_RGBA8888;
        let mut px = frame(f, 0, 0, 0);
        px[123] = SENTINEL;
        let o = run(&px, f);
        assert_eq!((o.verdict(), o.sentinel_left), (Some(Failure::Undrawn), 1));

        let mut px = frame(f, 0, 0, 0);
        px[456] &= 0x00ff_ffff;
        let o = run(&px, f);
        assert_eq!((o.verdict(), o.alpha_bad), (Some(Failure::Alpha), 1));

        let mut px = frame(f, 0, 0, 0);
        let last = px.len() - 1;
        px[last] = 0;
        let o = run(&px, f);
        assert_eq!((o.verdict(), o.guard_bad), (Some(Failure::Guard), 1));

        let px = frame(f, 0, 0, 0);
        let fr = Frame::new(&px, W, H, GUARD_ROWS, f).unwrap();
        let o = check(&fr, |_, _| [0, 0, 0, 0xff]);
        assert_eq!((o.verdict(), o.reads_bad), (Some(Failure::ReadPixels), 5));
    }

    /// A GL reading the rows the wrong way up is caught: at the flipped
    /// rows the frame has FILL, not the reference.
    #[test]
    fn flipped_readpixels_is_caught() {
        let f = FORMAT_RGBA8888;
        let px = frame(f, 0, 0, 0);
        let fr = Frame::new(&px, W, H, GUARD_ROWS, f).unwrap();
        let o = check(&fr, |x, y| fr.rgba(fr.row(y)[x as usize]));
        assert_eq!(o.verdict(), Some(Failure::ReadPixels));
        assert_eq!(o.reads_bad, 5);
    }

    /// The fingerprint is the CRC of the rows as the eye sees them: the
    /// same picture stored top-down or bottom-up gives one CRC, and that
    /// CRC is zlib's over the top-down bytes.
    #[test]
    fn crc_is_of_the_rows_top_down() {
        let top = frame(FORMAT_RGBA8888, 0, 0, 0);
        let bottom = frame(FORMAT_RGBA8888 | FORMAT_BOTTOM_UP, 0, 0, 0);
        let a = run(&top, FORMAT_RGBA8888).crc32;
        let b = run(&bottom, FORMAT_RGBA8888 | FORMAT_BOTTOM_UP).crc32;
        assert_eq!(a, b);
        let bytes: Vec<u8> = top[..(W * H) as usize].iter().flat_map(|p| p.to_le_bytes()).collect();
        assert_eq!(a, record::crc32_ieee_update(0, &bytes));
        let o = run(&top, FORMAT_RGBA8888);
        assert_eq!(o.fingerprint_words(W, H), [a, 2, (1024 << 16) | 600]);
    }

    /// PROBES' words are record.rs's packing of the memory points.
    #[test]
    fn probes_record_words() {
        let o = run(&frame(FORMAT_XRGB8888, 0, 0, 0), FORMAT_XRGB8888);
        let mut rgb = [[0u8; 3]; 5];
        for (o, p) in rgb.iter_mut().zip(REFERENCE.iter()) {
            *o = p.want;
        }
        assert_eq!(o.probes_words(), record::probes_words(&rgb));
        assert_eq!(o.probes_words()[0] & 0xffff, record::rgb565([11, 16, 23]) as u32);
    }

    /// `px` (packed rows of W, the window then the guard rows) laid out
    /// with rows `stride` words apart, the padding filled with `pad`.
    fn padded(px: &[u32], stride: u32, pad: u32) -> Vec<u32> {
        let rows = px.len() / W as usize;
        let mut out = vec![pad; (rows - 1) * stride as usize + W as usize];
        for (r, row) in px.chunks(W as usize).enumerate() {
            out[r * stride as usize..r * stride as usize + W as usize].copy_from_slice(row);
        }
        out
    }

    /// A window that is a view into a wider buffer (its rows `stride` words
    /// apart, CONFIG_YAKOGL_BOARD_SCANOUT_VIEW) checks exactly as the same
    /// picture packed: the same points, verdict and CRC (the fingerprint is
    /// over the window's W pixels a row, never the padding), and a padding
    /// word changed changes nothing.
    #[test]
    fn a_view_with_a_wider_stride_checks_as_the_packed_window() {
        for format in [FORMAT_XRGB8888, FORMAT_RGBA8888 | FORMAT_BOTTOM_UP] {
            let packed = frame(format, 0, 0, 0);
            let want = run(&packed, format);
            let stride = W + 256; /* the Pi 4: 1024 in a 1280-wide framebuffer */
            let mut view = padded(&packed, stride, 0x1234_5678);
            let check_view = |px: &[u32]| {
                let f = Frame::with_stride(px, W, H, stride, GUARD_ROWS, format).unwrap();
                check(&f, |x, y| f.rgba(f.row(H - 1 - y)[x as usize]))
            };
            let got = check_view(&view);
            assert_eq!((got.verdict(), got.crc32, got.points), (want.verdict(), want.crc32, want.points));
            assert_eq!((got.guard_bad, got.sentinel_left, got.alpha_bad), (0, 0, 0));
            /* a padding word (row 3, past the window) changed: nothing moves */
            view[3 * stride as usize + W as usize + 5] = SENTINEL;
            let again = check_view(&view);
            assert_eq!((again.crc32, again.verdict(), again.sentinel_left), (want.crc32, None, 0));
            /* the guard rows as the screen held them, never painted: any
             * contents pass while unchanged, one changed word fails */
            let rows = H + GUARD_ROWS;
            let mut screen = view.clone();
            let mut before = Vec::new();
            for g in 0..GUARD_ROWS {
                let start = ((H + g) * stride) as usize;
                for x in 0..W as usize {
                    screen[start + x] = 0x00a0_0000 + (g * W) + x as u32; /* console text, say */
                }
                before.extend_from_slice(&screen[start..start + W as usize]);
            }
            let snap = |px: &[u32]| {
                let f = Frame::with_stride(px, W, H, stride, GUARD_ROWS, format).unwrap().with_guard(&before).unwrap();
                check(&f, |x, y| f.rgba(f.row(H - 1 - y)[x as usize])).guard_bad
            };
            assert_eq!(snap(&screen), 0, "unchanged screen rows pass");
            screen[((rows - 1) * stride + 7) as usize] ^= 0x10;
            assert_eq!(snap(&screen), 1, "one guard word the GPU changed is caught");
            /* a window word changed: the CRC does move (the instrument sees it) */
            view[3 * stride as usize + 5] ^= 1;
            assert_ne!(check_view(&view).crc32, want.crc32);
        }
    }

    #[test]
    fn frame_refusals() {
        let px = vec![0u32; (W * (H + GUARD_ROWS)) as usize];
        assert_eq!(Frame::new(&px, W, H, GUARD_ROWS, 3).err(), Some(FrameError::Format));
        assert_eq!(Frame::new(&px[1..], W, H, GUARD_ROWS, 1).err(), Some(FrameError::Short));
        assert_eq!(Frame::new(&px, 300, 300, 0, 1).err(), Some(FrameError::Size));
        assert_eq!(Frame::new(&px, 0, H, 0, 1).err(), Some(FrameError::Size));
        assert_eq!(Frame::with_stride(&px, W, H, W - 1, GUARD_ROWS, 1).err(), Some(FrameError::Stride));
        assert_eq!(Frame::with_stride(&px, W, H, W + 2, GUARD_ROWS, 1).err(), Some(FrameError::Short));
    }
}
