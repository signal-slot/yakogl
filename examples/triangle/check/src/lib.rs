// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The triangle of the triangle examples (examples/triangle, whose GL calls
//! are C, and examples/triangle-rust, whose calls are Rust) and its check,
//! the same on the Raspberry Pi 4 (V3D), the Verdin AM62P (PowerVR) and the
//! Verdin iMX8M Plus (GC7000) (driver/docs/one-method.md, section 5, step 3).
//!
//! One definition of what is drawn and what is expected:
//!
//! - [`VERTS`], [`CLEAR`], [`VS_SRC`] and [`FS_SRC`] are what both examples
//!   draw (the C main takes them from here through its staticlib, ../rust;
//!   triangle-rust uses them directly), so the draw and the reference cannot
//!   drift apart. The shader texts are GLSL ES
//!   1.00, tools/shaders/gl_test_es100/passthrough.vert and interp_color.frag
//!   byte for byte (shaders.rs); YakoGL's online compiler builds them in a
//!   host test, and the tests here say which offline table holds them.
//! - [`check`] is a whole-window classification: every pixel is put in one
//!   of three classes by its distance from
//!   the triangle's edges at the pixel centre, in GL window coordinates:
//!
//!   ```text
//!   inside  (at least EDGE_PX inside every edge): the interpolated colour,
//!           within COLOR_TOL per channel, alpha 255
//!   outside (at least EDGE_PX outside some edge): exactly the clear colour
//!   edge    (the band between): the clear colour, or within COLOR_TOL of
//!           the interpolated colour with alpha 255
//!   ```
//!
//!   Then the guard rows after the window must hold [`GUARD`], and
//!   glReadPixels must agree with memory at [`read_points`] and at the
//!   PICTURE record's column (which checks the read path and the window's
//!   orientation).
//! - [`picture_words`] is the PICTURE record (kind 4 in record.rs): the
//!   centre pixel and a CRC of the centre column, both from glReadPixels.
//!
//! The guard is [`GUARD_ROWS`] rows after the window's memory (the board
//! layer gives the window). The window may be XRGB8888 and bottom-up (the
//! board's choice), so memory is read through [`Frame`]. The edge distance is
//! compared squared, which needs no `sqrt` (not in `core`).
#![cfg_attr(not(test), no_std)]

/// The result record, the one definition (the board layer's), compiled into
/// this crate as its header says to (no second copy).
#[path = "../../../../driver/rust/src/board/record.rs"]
pub mod record;

/// The window, one size on every board: the AM62P panel's, which fits all
/// three panels (1024x600, 1280x800, 1920x720), and the size of the
/// i.MX8MP's PICTURE baseline (record.rs, writer_reproduces_the_baseline_picture).
pub const WIDTH: u32 = 1024;
pub const HEIGHT: u32 = 600;
/// Rows after the window's memory that the GPU must not touch.
pub const GUARD_ROWS: u32 = 8;
/// Written by the CPU into every window pixel before the frame. As a
/// little-endian word it is magenta in both RGBA8888 and XRGB8888 memory.
pub const SENTINEL: u32 = 0xffff_00ff;
/// Written by the CPU into the guard rows.
pub const GUARD: u32 = 0xff00_00ff;

/// `struct gles_native_window.format` values (include/EGL/eglplatform.h).
pub const FORMAT_RGBA8888: u32 = 1;
pub const FORMAT_XRGB8888: u32 = 2;
pub const FORMAT_BOTTOM_UP: u32 = 0x8000_0000;

/// x, y, z, w, r, g, b, a: clip space, w = 1. Red top, green bottom left,
/// blue bottom right (every old triangle sample's).
pub const VERTS: [[f32; 8]; 3] = [
    [0.0, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
    [-0.8, -0.8, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
    [0.8, -0.8, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0],
];
/// glClearColor: a grey. 0.125 * 255 = 31.875, which every
/// backend's clear packing rounds to 0x20 (src/pvr/backend.rs pack_rgba8,
/// src/vivante/backend.rs unorm8, src/v3d/state.rs pack_rgba8; not a tie,
/// so round-half-even and round-half-up agree). Grey is the same bytes in
/// RGBA8888 and XRGB8888.
pub const CLEAR: [f32; 4] = [0.125, 0.125, 0.125, 1.0];
/// [`CLEAR`] as R, G, B, A bytes.
pub const CLEAR_RGBA: [u8; 4] = [0x20, 0x20, 0x20, 0xff];
/// The width of the edge band, in pixels, on each side of an edge.
pub const EDGE_PX: f32 = 1.0;
/// Per channel, 8-bit: v3d_triangle's tolerance (the largest difference
/// measured on the V3D was 1).
pub const COLOR_TOL: u8 = 3;

mod shaders;
/// The shader texts (shaders.rs): GLSL ES 1.00, NUL-terminated.
pub use shaders::{FS_SRC, VS_SRC};

/// GL window coordinates (origin bottom left) of the four glReadPixels
/// points for a `w` x `h` window.
pub fn read_points(w: u32, h: u32) -> [(u32, u32); 4] {
    [(1, 1), (w / 2, h / 2), (w / 2, h * 8 / 10), (w * 3 / 10, h * 2 / 10)]
}

/// The number of column samples in PICTURE's CRC.
pub const PICTURE_SAMPLES: usize = (record::PICTURE_LAST - record::PICTURE_FIRST + 1) as usize;

/// GL window coordinates of PICTURE's column samples, ascending:
/// x = W/2, y = i * H / 24 for i = 3..21 (record.rs, kind PICTURE).
pub fn picture_column(w: u32, h: u32) -> [(u32, u32); PICTURE_SAMPLES] {
    let mut out = [(0, 0); PICTURE_SAMPLES];
    for (k, o) in out.iter_mut().enumerate() {
        let i = record::PICTURE_FIRST + k as u32;
        *o = (w / 2, i * h / record::PICTURE_ROWS);
    }
    out
}

/// PICTURE's w1..w3 from glReadPixels results: `centre` at (W/2, H/2) and
/// `column` at [`picture_column`], each as glReadPixels' R, G, B, A bytes.
/// w1 is the centre's bytes as a little-endian word; w2 the CRC-32/IEEE over
/// the column's bytes in order; w3 = W << 16 | H.
pub fn picture_words(centre: [u8; 4], column: &[[u8; 4]; PICTURE_SAMPLES], w: u32, h: u32) -> [u32; 3] {
    let mut bytes = [0u8; 4 * PICTURE_SAMPLES];
    for (k, px) in column.iter().enumerate() {
        bytes[4 * k..4 * k + 4].copy_from_slice(px);
    }
    [u32::from_le_bytes(centre), record::crc32_ieee_update(0, &bytes), record::size_word(w, h)]
}

/// The triangle in GL window coordinates for a `w` x `h` viewport.
struct Tri {
    x: [f32; 3],
    y: [f32; 3],
    /// Twice the signed area.
    area2: f32,
    /// Each edge's squared length, edge i being the one opposite vertex i.
    len2: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Inside,
    Outside,
    Edge,
}

impl Tri {
    fn new(w: u32, h: u32) -> Self {
        let mut x = [0f32; 3];
        let mut y = [0f32; 3];
        for i in 0..3 {
            x[i] = (VERTS[i][0] + 1.0) * 0.5 * w as f32;
            y[i] = (VERTS[i][1] + 1.0) * 0.5 * h as f32;
        }
        let area2 = (x[1] - x[0]) * (y[2] - y[0]) - (y[1] - y[0]) * (x[2] - x[0]);
        let mut len2 = [0f32; 3];
        for (i, l) in len2.iter_mut().enumerate() {
            let (a, b) = ((i + 1) % 3, (i + 2) % 3);
            *l = (x[b] - x[a]) * (x[b] - x[a]) + (y[b] - y[a]) * (y[b] - y[a]);
        }
        Tri { x, y, area2, len2 }
    }

    /// The class of the point (x, y) and its barycentric weights.
    /// With the signed edge distance d = s * e / len (s the winding's sign),
    /// the point is inside for d >= EDGE_PX on every edge and outside for
    /// d <= -EDGE_PX on some edge. The comparisons are made as s * e against
    /// EDGE_PX * len, squared.
    fn at(&self, x: f32, y: f32, lambda: &mut [f32; 3]) -> Class {
        let (mut all_in, mut any_out) = (true, false);
        for (i, l) in lambda.iter_mut().enumerate() {
            let (a, b) = ((i + 1) % 3, (i + 2) % 3);
            let e = (self.x[b] - self.x[a]) * (y - self.y[a]) - (self.y[b] - self.y[a]) * (x - self.x[a]);
            *l = e / self.area2;
            let se = if self.area2 > 0.0 { e } else { -e };
            let far = se * se >= EDGE_PX * EDGE_PX * self.len2[i];
            if !(se >= 0.0 && far) {
                all_in = false;
            }
            if se <= 0.0 && far {
                any_out = true;
            }
        }
        if all_in {
            Class::Inside
        } else if any_out {
            Class::Outside
        } else {
            Class::Edge
        }
    }
}

/// The interpolated colour for barycentric weights, R, G, B, A = 255.
fn expected_rgba(lambda: &[f32; 3]) -> [u8; 4] {
    let mut out = [0, 0, 0, 0xff];
    for (c, o) in out.iter_mut().take(3).enumerate() {
        let mut v = 0f32;
        for (i, l) in lambda.iter().enumerate() {
            v += l * VERTS[i][4 + c];
        }
        let v = v.clamp(0.0, 1.0);
        *o = (v * 255.0 + 0.5) as u8;
    }
    out
}

/// The largest difference of the four channels.
fn max_diff(a: [u8; 4], b: [u8; 4]) -> u8 {
    (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0)
}

/// The expected colour at GL pixel (x, y) of a `w` x `h` window drawn
/// correctly: the clear colour outside, the interpolated colour elsewhere
/// (the edge band accepts either). For synthetic frames and the console.
pub fn reference_at(w: u32, h: u32, x: u32, y: u32) -> [u8; 4] {
    let t = Tri::new(w, h);
    let mut lambda = [0f32; 3];
    match t.at(x as f32 + 0.5, y as f32 + 0.5, &mut lambda) {
        Class::Outside => CLEAR_RGBA,
        _ => expected_rgba(&lambda),
    }
}

/// Why a frame could not be checked at all (not a verdict on the frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Smaller than the read points need (2 x 2).
    Size,
    /// Not RGBA8888 or XRGB8888 (with or without the bottom-up flag).
    Format,
    /// Fewer pixels than (h + guard rows) * w.
    Short,
}

impl FrameError {
    pub fn what(self) -> &'static str {
        match self {
            FrameError::Size => "the window is smaller than 2x2",
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
        if w < 2 || h < 2 {
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

    /// The memory row that holds GL row `gy` (GL counts up from the bottom;
    /// a top-down window keeps the top row first, a bottom-up one the
    /// bottom row).
    fn mem_row(&self, gy: u32) -> u32 {
        if self.bottom_up {
            gy
        } else {
            self.h - 1 - gy
        }
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

    /// The pixel at GL window coordinates (x, gy), R, G, B, A.
    pub fn at_gl(&self, x: u32, gy: u32) -> [u8; 4] {
        self.rgba(self.pixels[self.mem_row(gy) as usize * self.w as usize + x as usize])
    }

    /// The window's size in pixels (the guard rows not counted).
    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// The guard rows' words, as memory holds them.
    pub fn guard(&self) -> &'a [u32] {
        let start = self.h as usize * self.w as usize;
        &self.pixels[start..start + self.guard_rows as usize * self.w as usize]
    }
}

/// Which part of the check failed first, in the check's order. The
/// value is the APP record's stage byte for a failed check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Failure {
    Counts = 1,
    Inside = 2,
    Outside = 3,
    Edge = 4,
    Guard = 5,
    ReadPixels = 6,
}

impl Failure {
    pub fn what(self) -> &'static str {
        match self {
            Failure::Counts => "the pixel classes do not add up",
            Failure::Inside => "inside colour",
            Failure::Outside => "outside not the clear colour",
            Failure::Edge => "edge pixels",
            Failure::Guard => "the guard rows after the window changed",
            Failure::ReadPixels => "glReadPixels differs from memory",
        }
    }
}

/// The first wrong pixel: GL window coordinates, what memory holds and what
/// was expected (R, G, B, A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wrong {
    pub x: u32,
    pub y: u32,
    pub got: [u8; 4],
    pub want: [u8; 4],
}

/// Everything the check measured; the counts are over the whole window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub total: u32,
    pub n_in: u32,
    pub n_out: u32,
    pub n_edge: u32,
    /// Edge pixels holding the interpolated colour (the rest hold the clear
    /// colour or are wrong).
    pub edge_drawn: u32,
    pub bad_in: u32,
    pub bad_out: u32,
    pub bad_edge: u32,
    /// The largest channel difference over the inside pixels.
    pub worst_in: u8,
    pub first_wrong: Option<Wrong>,
    pub guard_bad: u32,
    /// glReadPixels against memory: points read, points different.
    pub reads: u32,
    pub reads_bad: u32,
    /// The first point where glReadPixels differed: (x, y) and the GL value
    /// (memory's is `at_gl`).
    pub first_read_bad: Option<(u32, u32, [u8; 4])>,
    /// PICTURE's w1..w3.
    pub picture: [u32; 3],
}

impl Outcome {
    /// The first failure, or None for a pass.
    pub fn verdict(&self) -> Option<Failure> {
        let counted = self.n_in + self.n_out + self.n_edge == self.total && self.n_in > 0 && self.n_out > 0;
        if !counted {
            Some(Failure::Counts)
        } else if self.bad_in != 0 {
            Some(Failure::Inside)
        } else if self.bad_out != 0 {
            Some(Failure::Outside)
        } else if self.bad_edge != 0 {
            Some(Failure::Edge)
        } else if self.guard_bad != 0 {
            Some(Failure::Guard)
        } else if self.reads_bad != 0 {
            Some(Failure::ReadPixels)
        } else {
            None
        }
    }
}

/// Check `frame`. `read_gl(x, y)` is glReadPixels of one RGBA pixel at GL
/// window coordinates (origin bottom left), with the frame's context
/// current.
pub fn check(frame: &Frame, mut read_gl: impl FnMut(u32, u32) -> [u8; 4]) -> Outcome {
    let (w, h) = (frame.w, frame.h);
    let t = Tri::new(w, h);
    let mut o = Outcome {
        total: 0,
        n_in: 0,
        n_out: 0,
        n_edge: 0,
        edge_drawn: 0,
        bad_in: 0,
        bad_out: 0,
        bad_edge: 0,
        worst_in: 0,
        first_wrong: None,
        guard_bad: 0,
        reads: 0,
        reads_bad: 0,
        first_read_bad: None,
        picture: [0; 3],
    };
    for gy in 0..h {
        let fy = gy as f32 + 0.5;
        for x in 0..w {
            let mut lambda = [0f32; 3];
            let class = t.at(x as f32 + 0.5, fy, &mut lambda);
            let got = frame.at_gl(x, gy);
            let want = expected_rgba(&lambda);
            o.total += 1;
            let wrong = match class {
                Class::Inside => {
                    o.n_in += 1;
                    let md = max_diff(got, want);
                    o.worst_in = o.worst_in.max(md);
                    let bad = md > COLOR_TOL || got[3] != 0xff;
                    o.bad_in += bad as u32;
                    bad.then_some(want)
                }
                Class::Outside => {
                    o.n_out += 1;
                    let bad = got != CLEAR_RGBA;
                    o.bad_out += bad as u32;
                    bad.then_some(CLEAR_RGBA)
                }
                Class::Edge => {
                    o.n_edge += 1;
                    if got == CLEAR_RGBA {
                        None
                    } else if max_diff(got, want) <= COLOR_TOL && got[3] == 0xff {
                        o.edge_drawn += 1;
                        None
                    } else {
                        o.bad_edge += 1;
                        Some(want)
                    }
                }
            };
            if let (Some(want), None) = (wrong, o.first_wrong) {
                o.first_wrong = Some(Wrong { x, y: gy, got, want });
            }
        }
    }
    o.guard_bad = frame.guard().iter().filter(|&&px| px != GUARD).count() as u32;

    let mut read = |x: u32, y: u32, o: &mut Outcome| -> [u8; 4] {
        let gl = read_gl(x, y);
        o.reads += 1;
        if gl != frame.at_gl(x, y) {
            o.reads_bad += 1;
            if o.first_read_bad.is_none() {
                o.first_read_bad = Some((x, y, gl));
            }
        }
        gl
    };
    let mut centre = [0u8; 4];
    for (i, &(x, y)) in read_points(w, h).iter().enumerate() {
        let gl = read(x, y, &mut o);
        if i == 1 {
            centre = gl; /* (W/2, H/2): PICTURE's centre */
        }
    }
    let mut column = [[0u8; 4]; PICTURE_SAMPLES];
    for (c, &(x, y)) in column.iter_mut().zip(picture_column(w, h).iter()) {
        *c = read(x, y, &mut o);
    }
    o.picture = picture_words(centre, &column, w, h);
    o
}

/// R, G, B, A bytes as one word, R in the low byte (how the report prints a
/// pixel).
fn word(p: [u8; 4]) -> u32 {
    u32::from_le_bytes(p)
}

/// Write the check's report: what was measured, the PICTURE words and the
/// verdict, one line per `\n`, each line starting with `name: `. Both
/// examples print it (examples/triangle and examples/triangle-rust), so the
/// console of a run reads the same whichever language made the GL calls.
pub fn report(out: &mut impl core::fmt::Write, name: &str, frame: &Frame, o: &Outcome) -> core::fmt::Result {
    let (w, h, guard_rows) = (frame.w, frame.h, frame.guard_rows);
    writeln!(
        out,
        "{name}: window {w}x{h}: {} pixels = {} inside + {} outside + {} edge (band {EDGE_PX} px)",
        o.total, o.n_in, o.n_out, o.n_edge
    )?;
    writeln!(out, "{name}:   inside wrong {} (largest channel diff {}, tolerance {COLOR_TOL})", o.bad_in, o.worst_in)?;
    writeln!(out, "{name}:   outside wrong {}, edge wrong {} (edge drawn {})", o.bad_out, o.bad_edge, o.edge_drawn)?;
    if let Some(f) = o.first_wrong {
        writeln!(
            out,
            "{name}:   first wrong at GL {},{}: 0x{:08x}, expected 0x{:08x} (R in the low byte)",
            f.x,
            f.y,
            word(f.got),
            word(f.want)
        )?;
    }
    writeln!(out, "{name}:   guard rows: {} of {} pixels changed", o.guard_bad, w * guard_rows)?;
    writeln!(out, "{name}:   glReadPixels against memory: {} of {} points differ", o.reads_bad, o.reads)?;
    if let Some((x, y, gl)) = o.first_read_bad {
        writeln!(
            out,
            "{name}:   first at GL {x},{y}: glReadPixels 0x{:08x}, memory 0x{:08x}",
            word(gl),
            word(frame.at_gl(x, y))
        )?;
    }
    let [p1, p2, p3] = o.picture;
    writeln!(
        out,
        "{name}: picture: centre 0x{p1:08x}, column crc 0x{p2:08x} ({PICTURE_SAMPLES} samples), size 0x{p3:08x}"
    )?;
    match o.verdict() {
        None => writeln!(out, "{name}: check PASS (every pixel read back)"),
        Some(f) => writeln!(out, "{name}: check FAIL: {} (code {})", f.what(), f as u8),
    }
}

#[cfg(feature = "printk")]
pub mod console;

#[cfg(test)]
mod tests;
