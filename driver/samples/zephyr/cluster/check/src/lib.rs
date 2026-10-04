// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The check and the records of the cluster sample
//! (driver/samples/zephyr/cluster), the same on the Raspberry Pi 4 (V3D), the
//! Verdin AM62P (PowerVR) and the Verdin iMX8M Plus (GC7000)
//! (driver/docs/one-method.md, section 5, step 5).
//!
//! The check is `v3d_gl_cluster/src/main.c`'s (step 3 there), moved here as
//! Rust. In the checked frame (frame 1 at Slint clock 0 ms, [`FRAME_TICKS`]
//! telemetry periods, frame 2 at [`FRAME2_MS`], frame 3 at [`FRAME3_MS`],
//! one swap) it checks:
//!
//! - each probe point of the host reference for the window's size
//!   ([`REFERENCES`]) is within [`TOLERANCE`] per channel;
//! - no pixel keeps the CPU's [`SENTINEL`], and every alpha byte is 255;
//! - the [`GUARD_ROWS`] rows after the window still hold [`GUARD`];
//! - glReadPixels agrees with memory at every probe point.
//!
//! **Why the table is per size.** The cluster scales itself to the window
//! (app-window.slint:110-129, `fit-width`): it keeps
//! `fit = clamp(720 * w / h, 1605, 1920)` of its 1920x720 design, centred,
//! scaled by `min(w / fit, h / 720)`. On the three panels that is scale 1
//! (1920x720), 0.797 (1280x800) and 0.638 (1024x600), the last two
//! letterboxed. A design point lands on a different window pixel, with a
//! different anti-aliased neighbourhood, on each panel, so each panel has
//! its own probe points, picked by the host reference at that size
//! (`host/ref/run.sh 1024x600 1280x800 1920x720`, which also writes
//! src/references.rs). Slint's software renderer, which draws the
//! reference, ignores transform-scale; host/ref applies AppWindow's scale as
//! the window's scale factor instead, so the reference is the scaled
//! picture the board draws (the first 1024x600 table, drawn unscaled, missed
//! the AM62P's correct frame at all 35 points). A window with no table is a
//! failure that says so ([`Failure::NoReference`]), never a silent pass.
//!
//! **The tolerance** is v3d_gl_cluster's 12 per channel: the reference is
//! Slint's software renderer on the host, the board draws with FemtoVG, and
//! the points sit in flat areas where the two must still agree. 12 was
//! measured on the V3D at 1920x720 and 1280x720 (v3d_gl_cluster README,
//! CLUSTER1); on the PowerVR and the GC7000, and at 1024x600 and 1280x800,
//! it is unmeasured until their first board rows.
//!
//! The records ([`Outcome::probes_words`], [`Outcome::fingerprint_words`],
//! [`Present::perf_words`], [`Present::app_words`]) use record.rs's kinds as
//! they are; nothing is added to the kind table.
#![cfg_attr(not(test), no_std)]

use core::fmt;

/// The result record, the one definition (the board layer's), compiled into
/// this crate as its header says to (no second copy).
#[path = "../../../../../rust/src/board/record.rs"]
pub mod record;

mod references;
pub use references::REFERENCES;

/// Rows after the window's memory that the GPU must not touch.
pub const GUARD_ROWS: u32 = 8;
/// Written by the CPU into every window pixel before the checked frame. As a
/// little-endian word it is magenta in both RGBA8888 and XRGB8888 memory.
pub const SENTINEL: u32 = 0xffff_00ff;
/// Written by the CPU into the guard rows.
pub const GUARD: u32 = 0xff00_00ff;
/// Per channel, 8-bit, against the host reference (see the module text).
pub const TOLERANCE: u8 = 12;
/// The checked frame's sequence, the host reference's too
/// (host/ref/src/main.rs uses the same three numbers).
pub const FRAME_TICKS: u32 = 400;
pub const FRAME2_MS: u64 = 1000;
pub const FRAME3_MS: u64 = 3000;
/// The frames that make up the checked window (FINGERPRINT's w2).
pub const FRAMES: u32 = 3;
/// The cluster's telemetry period (its sim runs at 20 Hz).
pub const TICK_MS: u64 = 50;
/// The longest an animation frame may take, from its start to its end; a
/// longer one fails the run (v3d_gl_cluster's frame budget).
pub const FRAME_BUDGET_MS: u32 = 1000;
/// The thumbnail's block size, in pixels (host/ref/compare.py reads it).
pub const THUMB: u32 = 20;
/// The largest probe table any size may have.
pub const MAX_PROBES: usize = 64;

/// `struct gles_native_window.format` values (include/EGL/eglplatform.h).
pub const FORMAT_RGBA8888: u32 = 1;
pub const FORMAT_XRGB8888: u32 = 2;
pub const FORMAT_BOTTOM_UP: u32 = 0x8000_0000;

/// A probe point of the host reference: window coordinates (y down) and the
/// reference's R, G, B there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    pub x: u16,
    pub y: u16,
    pub rgb: [u8; 3],
}

/// The probe points of one window size.
#[derive(Clone, Copy, Debug)]
pub struct Reference {
    pub w: u32,
    pub h: u32,
    pub probes: &'static [Probe],
}

impl Reference {
    /// The table for a `w` x `h` window, if the host reference was run there.
    pub fn for_size(w: u32, h: u32) -> Option<&'static Reference> {
        REFERENCES.iter().find(|r| r.w == w && r.h == h)
    }

    /// The five probes PROBES carries: spread over the table (it is in the
    /// reference's grid order, top rows first), index `i * (n - 1) / 4`.
    /// Fixed per size, so two runs at one size compare word for word.
    pub fn record_points(&self) -> [usize; 5] {
        let last = self.probes.len().saturating_sub(1);
        core::array::from_fn(|i| i * last / 4)
    }
}

/// Why a frame could not be checked at all (not a verdict on the frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A zero size.
    Size,
    /// Not RGBA8888 or XRGB8888 (with or without the bottom-up flag).
    Format,
    /// Fewer pixels than (h + guard rows) * w.
    Short,
}

impl FrameError {
    pub fn what(self) -> &'static str {
        match self {
            FrameError::Size => "the window has a zero size",
            FrameError::Format => "the window format is neither RGBA8888 nor XRGB8888",
            FrameError::Short => "the window memory is shorter than the window and its guard rows",
        }
    }
}

/// The window's memory as the check sees it.
pub struct Frame<'a> {
    pixels: &'a [u32],
    w: u32,
    h: u32,
    guard_rows: u32,
    xrgb: bool,
    bottom_up: bool,
}

impl<'a> Frame<'a> {
    /// `pixels` holds `(h + guard_rows) * w` words, rows tightly packed, the
    /// guard rows after the window's last row in memory. `format` is the
    /// native window's format word as the board gave it.
    pub fn new(pixels: &'a [u32], w: u32, h: u32, guard_rows: u32, format: u32) -> Result<Self, FrameError> {
        if w == 0 || h == 0 {
            return Err(FrameError::Size);
        }
        let xrgb = match format & !FORMAT_BOTTOM_UP {
            FORMAT_RGBA8888 => false,
            FORMAT_XRGB8888 => true,
            _ => return Err(FrameError::Format),
        };
        let need = (h as usize + guard_rows as usize) * w as usize;
        if pixels.len() < need {
            return Err(FrameError::Short);
        }
        Ok(Frame { pixels, w, h, guard_rows, xrgb, bottom_up: format & FORMAT_BOTTOM_UP != 0 })
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    /// Row `y` as the eye sees it (0 = top). A bottom-up window keeps the
    /// bottom row first in memory.
    fn row(&self, y: u32) -> &[u32] {
        let mem_row = if self.bottom_up { self.h - 1 - y } else { y };
        let start = mem_row as usize * self.w as usize;
        &self.pixels[start..start + self.w as usize]
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

    /// The pixel at (x, y), y down, as R, G, B, A.
    pub fn at(&self, x: u32, y: u32) -> [u8; 4] {
        self.rgba(self.row(y)[x as usize])
    }
}

/// Which part of the check failed first, in v3d_gl_cluster's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    NoReference,
    Probes,
    Undrawn,
    Alpha,
    Guard,
    ReadPixels,
}

impl Failure {
    pub fn what(self) -> &'static str {
        match self {
            Failure::NoReference => "no host reference for this window size (check/src/references.rs)",
            Failure::Probes => "the host reference's probe points",
            Failure::Undrawn => "part of the window was not drawn",
            Failure::Alpha => "alpha is not 255 everywhere",
            Failure::Guard => "the guard rows after the window changed",
            Failure::ReadPixels => "glReadPixels differs from memory",
        }
    }
}

/// Everything the check measured; each count is over the whole window.
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    /// The table used, None when the size has none.
    pub reference: Option<&'static Reference>,
    /// Memory at the probe points, R, G, B, A (the first `n`).
    pub points: [[u8; 4]; MAX_PROBES],
    /// glReadPixels at the probe points, R, G, B, A.
    pub via_gl: [[u8; 4]; MAX_PROBES],
    /// The largest channel difference to the reference, per point.
    pub diff: [u8; MAX_PROBES],
    pub n: usize,
    pub worst: u8,
    pub probes_bad: u32,
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
        if self.reference.is_none() {
            Some(Failure::NoReference)
        } else if self.probes_bad != 0 {
            Some(Failure::Probes)
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

    /// PROBES' w1..w3: the five record points from memory (zeros when the
    /// size has no table; the status says FAIL then).
    pub fn probes_words(&self) -> [u32; 3] {
        let mut rgb = [[0u8; 3]; 5];
        if let Some(r) = self.reference {
            for (o, i) in rgb.iter_mut().zip(r.record_points()) {
                o.copy_from_slice(&self.points[i][..3]);
            }
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

/// Check `frame` against the table for its size. `read_gl(x, y)` is
/// glReadPixels of one RGBA pixel at GL window coordinates (origin bottom
/// left), with the frame's context current.
pub fn check(frame: &Frame, mut read_gl: impl FnMut(u32, u32) -> [u8; 4]) -> Outcome {
    let mut o = Outcome {
        reference: Reference::for_size(frame.w, frame.h),
        points: [[0; 4]; MAX_PROBES],
        via_gl: [[0; 4]; MAX_PROBES],
        diff: [0; MAX_PROBES],
        n: 0,
        worst: 0,
        probes_bad: 0,
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
    let guard_start = frame.h as usize * frame.w as usize;
    let guard_end = guard_start + frame.guard_rows as usize * frame.w as usize;
    o.guard_bad = frame.pixels[guard_start..guard_end].iter().filter(|&&px| px != GUARD).count() as u32;
    let Some(r) = o.reference else {
        return o;
    };
    o.n = r.probes.len().min(MAX_PROBES);
    for (i, p) in r.probes.iter().take(o.n).enumerate() {
        let (x, y) = (p.x as u32, p.y as u32);
        let mem = frame.at(x, y);
        let gl = read_gl(x, frame.h - 1 - y);
        o.points[i] = mem;
        o.via_gl[i] = gl;
        o.diff[i] = max_diff(mem, p.rgb);
        o.worst = o.worst.max(o.diff[i]);
        o.probes_bad += (o.diff[i] > TOLERANCE) as u32;
        o.reads_bad += (gl != mem) as u32;
    }
    o
}

/// The thumbnail's size in blocks: `(w / THUMB, h / THUMB)`.
pub fn thumb_blocks(frame: &Frame) -> (u32, u32) {
    (frame.w / THUMB, frame.h / THUMB)
}

/// One row of the thumbnail, v3d_gl_cluster's format, which
/// host/ref/compare.py reads: `thumb <row, 2 digits> <rrggbb per block>`,
/// each block the rounded mean of its THUMB x THUMB pixels.
pub fn write_thumb_row(out: &mut impl fmt::Write, frame: &Frame, by: u32) -> fmt::Result {
    write!(out, "thumb {by:02} ")?;
    for bx in 0..frame.w / THUMB {
        let mut sum = [0u32; 3];
        for y in by * THUMB..(by + 1) * THUMB {
            for x in bx * THUMB..(bx + 1) * THUMB {
                let p = frame.at(x, y);
                for (s, v) in sum.iter_mut().zip(p) {
                    *s += v as u32;
                }
            }
        }
        for s in sum {
            write!(out, "{:02x}", (s + THUMB * THUMB / 2) / (THUMB * THUMB))?;
        }
    }
    Ok(())
}

/// One animation frame's times, in microseconds:
/// - `draw`: slint_cluster_draw_frame (Slint, FemtoVG and the GL calls; the
///   GPU renders nothing of this frame yet);
/// - `wait`: until the previous frame's fence was seen signalled;
/// - `show`: board_show of the previous frame (a flip, or the Pi 4's copy);
/// - `swap`: eglSwapBuffers of this frame (its submit);
/// - `frame`: from this frame's start to its end (the four, and the rest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameTimes {
    pub draw: u32,
    pub wait: u32,
    pub show: u32,
    pub swap: u32,
    pub frame: u32,
}

/// The present accounting of the animation (v3d_gl_cluster's present_acc.h,
/// for this loop): the frames presented, and each part's sum and worst.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Present {
    pub presented: u32,
    pub sum: [u64; 5],
    pub worst: [u32; 5],
}

/// The order of [`Present::sum`] and [`Present::worst`]: [`FrameTimes`]'
/// fields.
pub const PART_NAMES: [&str; 5] = ["draw", "wait", "show", "swap", "frame"];
const DRAW: usize = 0;
const SHOW: usize = 2;
const SWAP: usize = 3;
const FRAME: usize = 4;

impl Present {
    /// One frame presented, with its times.
    pub fn add(&mut self, t: &FrameTimes) {
        self.presented += 1;
        for (i, v) in [t.draw, t.wait, t.show, t.swap, t.frame].into_iter().enumerate() {
            self.sum[i] += v as u64;
            self.worst[i] = self.worst[i].max(v);
        }
    }

    /// The mean of part `i` in microseconds (0 when nothing was presented).
    pub fn mean_us(&self, i: usize) -> u64 {
        if self.presented == 0 {
            0
        } else {
            self.sum[i] / self.presented as u64
        }
    }

    /// Frames presented per second over `elapsed_ms`, times 10 (a zero
    /// interval counts as 1 ms).
    pub fn fps_x10(&self, elapsed_ms: u32) -> u32 {
        (self.presented as u64 * 10_000 / elapsed_ms.max(1) as u64).min(u32::MAX as u64) as u32
    }

    /// PERF page 0 (tag 0: experiment 0, page 0), six 16-bit values, each
    /// saturating at 0xffff: fps x10; the frame's mean and worst; the mean
    /// draw, swap and show. Times in units of 0.1 ms.
    pub fn perf_words(&self, elapsed_ms: u32) -> [u32; 3] {
        let tenth = |us: u64| sat16(us / 100);
        let v = [
            sat16(self.fps_x10(elapsed_ms) as u64),
            tenth(self.mean_us(FRAME)),
            tenth(self.worst[FRAME] as u64),
            tenth(self.mean_us(DRAW)),
            tenth(self.mean_us(SWAP)),
            tenth(self.mean_us(SHOW)),
        ];
        [v[0] | v[1] << 16, v[2] | v[3] << 16, v[4] | v[5] << 16]
    }

    /// APP's w1..w3: `APP_SHOWN_BY_COPY (when board_show copied) | stage << 8
    /// | status`, the frames presented, and the milliseconds they were
    /// presented in (the kind leaves w3 to the application; the fps is
    /// w2 * 1000 / w3).
    pub fn app_words(&self, stage: u8, status: u32, shown_by_copy: bool, elapsed_ms: u32) -> [u32; 3] {
        let copy = if shown_by_copy { record::APP_SHOWN_BY_COPY } else { 0 };
        [copy | (stage as u32) << 8 | (status & 0xff), self.presented, elapsed_ms]
    }
}

fn sat16(v: u64) -> u32 {
    v.min(0xffff) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opaque colour no probe has.
    const FILL: [u8; 3] = [200, 7, 99];

    fn word(c: [u8; 3], xrgb: bool) -> u32 {
        if xrgb {
            u32::from_le_bytes([c[2], c[1], c[0], 0xff])
        } else {
            u32::from_le_bytes([c[0], c[1], c[2], 0xff])
        }
    }

    /// A synthetic drawn frame of the reference `r`: FILL everywhere, the
    /// reference values at its probes (probe `which` moved by `delta` on
    /// `channel`), guard rows intact.
    fn frame(r: &Reference, format: u32, which: usize, channel: usize, delta: i16) -> Vec<u32> {
        let xrgb = format & !FORMAT_BOTTOM_UP == FORMAT_XRGB8888;
        let mut px = vec![word(FILL, xrgb); (r.w * r.h) as usize];
        px.extend(core::iter::repeat_n(GUARD, (r.w * GUARD_ROWS) as usize));
        for (i, p) in r.probes.iter().enumerate() {
            let mut c = p.rgb;
            if i == which {
                c[channel] = (c[channel] as i16 + delta).clamp(0, 255) as u8;
            }
            let y = p.y as u32;
            let mem_row = if format & FORMAT_BOTTOM_UP != 0 { r.h - 1 - y } else { y };
            px[(mem_row * r.w + p.x as u32) as usize] = word(c, xrgb);
        }
        px
    }

    /// The check with glReadPixels as a correct GL answers it: GL row y is
    /// the eye's row h-1-y.
    fn run(px: &[u32], r: &Reference, format: u32) -> Outcome {
        let f = Frame::new(px, r.w, r.h, GUARD_ROWS, format).unwrap();
        check(&f, |x, y| f.at(x, r.h - 1 - y))
    }

    /// The three panels have a table, each probe lies inside its window and
    /// is not a duplicate, and no table outgrows the outcome.
    #[test]
    fn the_three_panels_have_a_table() {
        let want = [(1024, 600), (1280, 800), (1920, 720)];
        assert_eq!(REFERENCES.len(), want.len(), "a size was added or dropped: re-derive this pin");
        for (w, h) in want {
            let r = Reference::for_size(w, h).unwrap_or_else(|| panic!("no table for {w}x{h}"));
            assert!(r.probes.len() >= 5 && r.probes.len() <= MAX_PROBES, "{w}x{h}: {}", r.probes.len());
            for (i, p) in r.probes.iter().enumerate() {
                assert!((p.x as u32) < w && (p.y as u32) < h, "{w}x{h} probe {i}");
                assert!(!r.probes[..i].iter().any(|q| q.x == p.x && q.y == p.y), "{w}x{h} probe {i} twice");
            }
            let rp = r.record_points();
            assert_eq!((rp[0], rp[4]), (0, r.probes.len() - 1));
            assert!(rp.windows(2).all(|p| p[0] < p[1]), "{w}x{h}: {rp:?}");
        }
        // The counts host/ref/run.sh printed for these tables (2026-10-03,
        // the reference drawn at AppWindow's scale through the window's scale
        // factor: 15 and 19 flat points at 0.638 and 0.797, 34 at 1).
        let n: Vec<usize> = want.iter().map(|&(w, h)| Reference::for_size(w, h).unwrap().probes.len()).collect();
        assert_eq!(n, [15, 19, 34]);
        assert!(Reference::for_size(1280, 720).is_none());
    }

    #[test]
    fn reference_values_pass_in_every_format() {
        for r in REFERENCES {
            for format in [
                FORMAT_RGBA8888,
                FORMAT_XRGB8888,
                FORMAT_RGBA8888 | FORMAT_BOTTOM_UP,
                FORMAT_XRGB8888 | FORMAT_BOTTOM_UP,
            ] {
                let o = run(&frame(r, format, 0, 0, 0), r, format);
                assert_eq!(o.verdict(), None, "{}x{} format {format:#x}: {:?}", r.w, r.h, o.verdict());
                assert_eq!((o.n, o.worst, o.probes_bad, o.reads_bad), (r.probes.len(), 0, 0, 0));
            }
        }
    }

    /// The tolerance on every probe of every table and every channel: 12
    /// passes, 13 fails and counts the point (a step the clamp at 0 or 255
    /// would shorten is taken in the other direction).
    #[test]
    fn tolerance_is_twelve_per_channel() {
        let mut cases = 0;
        for r in REFERENCES {
            for which in 0..r.probes.len() {
                for channel in 0..3 {
                    let v = r.probes[which].rgb[channel] as i16;
                    // literals, not TOLERANCE: the test pins the number
                    for (step, pass) in [(12i16, true), (13, false)] {
                        let delta = if v + step <= 255 { step } else { -step };
                        let o = run(&frame(r, FORMAT_RGBA8888, which, channel, delta), r, FORMAT_RGBA8888);
                        let want = if pass { None } else { Some(Failure::Probes) };
                        assert_eq!(o.verdict(), want, "{}x{} probe {which} ch {channel} delta {delta}", r.w, r.h);
                        assert_eq!(o.diff[which], delta.unsigned_abs() as u8);
                        assert_eq!(o.probes_bad, (!pass) as u32);
                        cases += 1;
                    }
                }
            }
        }
        assert_eq!(cases, (15 + 19 + 34) * 3 * 2);
    }

    #[test]
    fn sentinel_alpha_guard_readpixels_and_size_each_fail() {
        let r = Reference::for_size(1024, 600).unwrap();
        let f = FORMAT_XRGB8888;
        let mut px = frame(r, f, 0, 0, 0);
        px[123] = SENTINEL;
        let o = run(&px, r, f);
        assert_eq!((o.verdict(), o.sentinel_left), (Some(Failure::Undrawn), 1));

        let mut px = frame(r, f, 0, 0, 0);
        px[456] &= 0x00ff_ffff;
        let o = run(&px, r, f);
        assert_eq!((o.verdict(), o.alpha_bad), (Some(Failure::Alpha), 1));

        let mut px = frame(r, f, 0, 0, 0);
        let last = px.len() - 1;
        px[last] = 0;
        let o = run(&px, r, f);
        assert_eq!((o.verdict(), o.guard_bad), (Some(Failure::Guard), 1));

        let px = frame(r, f, 0, 0, 0);
        let fr = Frame::new(&px, r.w, r.h, GUARD_ROWS, f).unwrap();
        let o = check(&fr, |_, _| [0, 0, 0, 0xff]);
        assert_eq!((o.verdict(), o.reads_bad), (Some(Failure::ReadPixels), r.probes.len() as u32));

        // A size with no table (the Pi 4's old 1280x720): every other part is
        // still checked, and the verdict names the missing table.
        let mut px = vec![word(FILL, true); 1280 * (720 + GUARD_ROWS as usize)];
        for g in px[1280 * 720..].iter_mut() {
            *g = GUARD;
        }
        let fr = Frame::new(&px, 1280, 720, GUARD_ROWS, f).unwrap();
        let o = check(&fr, |_, _| [0; 4]);
        assert_eq!((o.verdict(), o.n, o.guard_bad, o.sentinel_left), (Some(Failure::NoReference), 0, 0, 0));
        assert_eq!(o.probes_words(), record::probes_words(&[[0; 3]; 5]));
    }

    #[test]
    fn frame_refusals() {
        let px = vec![0u32; 64 * (32 + GUARD_ROWS as usize)];
        assert_eq!(Frame::new(&px, 64, 32, GUARD_ROWS, 3).err(), Some(FrameError::Format));
        assert_eq!(Frame::new(&px[1..], 64, 32, GUARD_ROWS, 1).err(), Some(FrameError::Short));
        assert_eq!(Frame::new(&px, 0, 32, 0, 1).err(), Some(FrameError::Size));
    }

    /// A GL reading the rows the wrong way up is caught.
    #[test]
    fn flipped_readpixels_is_caught() {
        let r = Reference::for_size(1920, 720).unwrap();
        let px = frame(r, FORMAT_RGBA8888, 0, 0, 0);
        let fr = Frame::new(&px, r.w, r.h, GUARD_ROWS, FORMAT_RGBA8888).unwrap();
        let o = check(&fr, |x, y| fr.at(x, y));
        assert_eq!(o.verdict(), Some(Failure::ReadPixels));
        assert!(o.reads_bad > 0);
    }

    /// The fingerprint is the CRC of the rows as the eye sees them, and
    /// PROBES carries the record points.
    #[test]
    fn records() {
        let r = Reference::for_size(1280, 800).unwrap();
        let top = frame(r, FORMAT_XRGB8888, 0, 0, 0);
        let bottom = frame(r, FORMAT_XRGB8888 | FORMAT_BOTTOM_UP, 0, 0, 0);
        let a = run(&top, r, FORMAT_XRGB8888);
        let b = run(&bottom, r, FORMAT_XRGB8888 | FORMAT_BOTTOM_UP);
        assert_eq!(a.crc32, b.crc32);
        let bytes: Vec<u8> = top[..(r.w * r.h) as usize].iter().flat_map(|p| p.to_le_bytes()).collect();
        assert_eq!(a.crc32, record::crc32_ieee_update(0, &bytes));
        assert_eq!(a.fingerprint_words(r.w, r.h), [a.crc32, 3, (1280 << 16) | 800]);
        let mut rgb = [[0u8; 3]; 5];
        for (o, i) in rgb.iter_mut().zip(r.record_points()) {
            *o = r.probes[i].rgb;
        }
        assert_eq!(a.probes_words(), record::probes_words(&rgb));
    }

    #[test]
    fn thumbnail_row_is_the_block_mean() {
        let (w, h) = (2 * THUMB, THUMB);
        let mut px = vec![word([10, 20, 30], true); (w * h) as usize];
        // block 0's red: (399 * 10 + 210) / 400 = 10.5, rounded to 11
        px[0] = word([210, 20, 30], true);
        let fr = Frame::new(&px, w, h, 0, FORMAT_XRGB8888).unwrap();
        assert_eq!(thumb_blocks(&fr), (2, 1));
        let mut s = String::new();
        write_thumb_row(&mut s, &fr, 0).unwrap();
        assert_eq!(s, "thumb 00 0b141e0a141e");
    }

    #[test]
    fn present_accounting_and_its_records() {
        let mut p = Present::default();
        assert_eq!((p.fps_x10(1000), p.mean_us(0)), (0, 0));
        for i in 0..60u32 {
            p.add(&FrameTimes { draw: 4000, wait: 1000 + i, show: 500, swap: 300, frame: 16_000 + i * 10 });
        }
        assert_eq!(p.presented, 60);
        assert_eq!(p.fps_x10(1000), 600);
        assert_eq!(p.fps_x10(0), 600_000, "a zero interval counts as 1 ms, never a division by zero");
        assert_eq!(p.worst[FRAME], 16_590);
        assert_eq!(p.mean_us(FRAME), 16_295);
        // 60.0 fps; frame mean 162 tenths of a ms, worst 165; draw 40; swap 3; show 5
        assert_eq!(p.perf_words(1000), [600 | 162 << 16, 165 | 40 << 16, 3 | 5 << 16]);
        assert_eq!(p.app_words(7, record::STATUS_PASS, false, 1000), [7 << 8 | 1, 60, 1000]);
        assert_eq!(p.app_words(7, record::STATUS_FAIL, true, 1000), [0x1_0000 | 7 << 8 | 2, 60, 1000]);
        // record.rs reads the draw back from these words (the one reader)
        let w = p.perf_words(1000);
        let recs = [
            [record::w0(record::kind::FINGERPRINT, 1), 0, 3, record::size_word(1024, 600)],
            [record::w0(record::kind::PERF, 0), w[0], w[1], w[2]],
            [record::w0(record::kind::APP, 3), 7 << 8 | 1, 60, 1000],
        ];
        let t = record::Throughput::from_records(&recs).unwrap();
        assert_eq!((t.draw_tenths, t.frames, t.ms, t.w, t.h), (Some(40), 60, 1000, 1024, 600));
        // saturation, not wrap
        let mut q = Present::default();
        q.add(&FrameTimes { frame: u32::MAX, ..Default::default() });
        assert_eq!((q.perf_words(1)[0] >> 16, q.perf_words(1)[1] & 0xffff), (0xffff, 0xffff));
    }
}
