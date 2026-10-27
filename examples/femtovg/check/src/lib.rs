// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The femtovg example's scene and its check (examples/femtovg), the same on
//! the Raspberry Pi 4 (V3D), the Verdin AM62P (PowerVR) and the Verdin
//! iMX8M Plus (GC7000).
//!
//! One definition of what is drawn and what is expected:
//!
//! - The scene: a clear to [`BG`], a rounded rect filled with [`RRECT`], a
//!   rect filled with a vertical linear gradient from [`GRAD_TOP`] to
//!   [`GRAD_BOTTOM`], and a polyline stroked with [`STROKE`]. The example's
//!   staticlib (../rust) draws it with femtovg from these constants, so the
//!   draw and the reference cannot drift apart. The geometry is the old
//!   gl_femtovg's (`fvg_demo`).
//! - [`check`] is v3d_gl_femtovg's whole-window classification
//!   (v3d_gl_femtovg/src/main.c, check_window). Every pixel is put in one
//!   class by its position, at the pixel centre, in canvas coordinates (x
//!   right, y down; femtovg's):
//!
//!   ```text
//!   rrect    at least FILL_MARGIN inside the rounded rect: RRECT
//!   grad     at least FILL_MARGIN inside the gradient rect: the colour
//!            mixed at the pixel centre, within GRAD_TOL (femtovg dithers
//!            gradients by up to half an 8-bit step, main-fs.glsl)
//!   stroke   within STROKE_CORE of a segment's centre line and at least
//!            STROKE_END_GAP from its ends: STROKE
//!   bg       clear of every shape by the margins: BG
//!   sfringe  0.2-0.8 of the way through the stroke's 1-px fringe
//!   rfringe  0.2-0.8 of the way through the rounded rect's fringe
//!   edge     everything else (other fringes, joins): counted only
//!   ```
//!
//!   The four solid classes must hold their colour within the tolerance and
//!   alpha 255. A fringe pixel must be drawn partly: each channel between
//!   [`BG`] and the shape's colour (within [`FRINGE_TOL`]), alpha 255, and
//!   more than FRINGE_TOL from BG. The stroke's fringe is drawn by
//!   femtovg's second stroke pass (stencil EQUAL 0 after a pass that
//!   discards and increments the stencil), the rounded rect's by a plain
//!   convex fill, the control. Then the guard rows after the window must
//!   hold [`GUARD`], and glReadPixels must agree with memory at five points
//!   and at PICTURE's column.
//! - PICTURE (record.rs kind 4) is the triangle's packing
//!   ([`triangle_check::picture_words`]): the centre pixel and a CRC of the
//!   centre column, both from glReadPixels. The Pi 4 and the AM62P stored
//!   the same words on 2026-10-25 (594b0407 ff201818 40e9c78c 04000258),
//!   join pixels included; the i.MX8MP's is not measured yet. PICTURE is
//!   not the verdict: the check is.
//!
//! The window reader ([`Frame`]), the guard and sentinel words and the
//! console are the triangle check's: the same window from the board layer.
#![cfg_attr(not(test), no_std)]

#[cfg(feature = "printk")]
pub use triangle_check::{console, say};
pub use triangle_check::{
    picture_column, picture_words, record, Frame, FrameError, FORMAT_BOTTOM_UP, FORMAT_RGBA8888, FORMAT_XRGB8888,
    GUARD, GUARD_ROWS, PICTURE_SAMPLES, SENTINEL,
};

/// The window, one size on every board (the triangle's): it fits all three
/// panels (1024x600, 1280x800, 1920x720).
pub const WIDTH: u32 = triangle_check::WIDTH;
pub const HEIGHT: u32 = triangle_check::HEIGHT;

/// The clear colour, R, G, B.
pub const BG: [u8; 3] = [24, 24, 32];
/// The rounded rect's fill.
pub const RRECT: [u8; 3] = [80, 160, 240];
/// The gradient at the top and at the bottom of its rect.
pub const GRAD_TOP: [u8; 3] = [240, 80, 120];
pub const GRAD_BOTTOM: [u8; 3] = [250, 210, 90];
/// The polyline's colour.
pub const STROKE: [u8; 3] = [120, 240, 160];

/// The rounded rect: x, y, width, height as fractions of the window.
pub const RRECT_BOX: [f32; 4] = [0.08, 0.10, 0.36, 0.30];
/// Its corner radius in pixels.
pub const RRECT_RADIUS: f32 = 16.0;
/// The gradient rect, as [`RRECT_BOX`]. The gradient runs from its top edge
/// to its bottom edge.
pub const GRAD_BOX: [f32; 4] = [0.54, 0.10, 0.38, 0.30];
/// The polyline's points, as fractions of the window.
pub const STROKE_PTS: [[f32; 2]; 5] = [[0.10, 0.75], [0.30, 0.55], [0.50, 0.80], [0.70, 0.55], [0.90, 0.75]];
/// Its width in pixels.
pub const STROKE_WIDTH: f32 = 6.0;

/// A box of fractions in pixels for a `w` x `h` window: x, y, width, height.
/// The draw and the check both scale through here.
pub fn box_px(b: [f32; 4], w: u32, h: u32) -> [f32; 4] {
    let (w, h) = (w as f32, h as f32);
    [w * b[0], h * b[1], w * b[2], h * b[3]]
}

/// A point of [`STROKE_PTS`] in pixels.
pub fn point_px(p: [f32; 2], w: u32, h: u32) -> [f32; 2] {
    [w as f32 * p[0], h as f32 * p[1]]
}

// femtovg's fringe is 1 px (fringe_width = 1 / dpi): the anti-aliased band of
// a fill is within 0.5 px of the path, of a stroke within 0.5 px of its outer
// edge; the margins keep every checked pixel out of it with room.
pub const FILL_MARGIN: f32 = 2.0;
/// femtovg's strokeMask is 1 up to 2.5 px off the centre line.
pub const STROKE_CORE: f32 = 1.5;
/// Joins (miter) and butt caps stay out of the stroke classes.
pub const STROKE_END_GAP: f32 = 12.0;
pub const BG_STROKE_GAP: f32 = STROKE_WIDTH / 2.0 + 3.0;
/// A miter tip at these angles is about 3.1 px; generous.
pub const BG_JOIN_GAP: f32 = 24.0;
/// The stroke fringe's band: femtovg's alpha is 3.5 - d over the outer pixel
/// of a width-6 stroke, so 2.7..3.3 px off the centre line is alpha 0.8..0.2.
pub const SFRINGE_FROM: f32 = 2.7;
pub const SFRINGE_TO: f32 = 3.3;
/// The fill's fringe is centred on the path: within 0.3 px of it is alpha
/// 0.2..0.8.
pub const RFRINGE_BAND: f32 = 0.3;
/// Per channel, 8-bit.
pub const EXACT_TOL: u8 = 1;
pub const GRAD_TOL: u8 = 2;
pub const FRINGE_TOL: u8 = 2;

/// The pixel classes, in report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    RRect,
    Grad,
    Stroke,
    Bg,
    StrokeFringe,
    RRectFringe,
    Edge,
}

pub const CLASSES: [Class; 7] =
    [Class::RRect, Class::Grad, Class::Stroke, Class::Bg, Class::StrokeFringe, Class::RRectFringe, Class::Edge];

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::RRect => "rrect",
            Class::Grad => "grad",
            Class::Stroke => "stroke",
            Class::Bg => "bg",
            Class::StrokeFringe => "sfringe",
            Class::RRectFringe => "rfringe",
            Class::Edge => "edge",
        }
    }

    fn fringe(self) -> bool {
        matches!(self, Class::StrokeFringe | Class::RRectFringe)
    }
}

/// `core` has no square root at the workspace's rust-version: Newton's
/// iteration in f64 from the exponent-halved guess (within 6%), five steps,
/// is exact to f64 precision, then rounded to f32.
fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let x = v as f64;
    let mut r = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..5 {
        r = 0.5 * (r + x / r);
    }
    r as f32
}

fn abs(v: f32) -> f32 {
    if v < 0.0 {
        -v
    } else {
        v
    }
}

/// femtovg's sdroundrect (main-fs.glsl) for a rect at (x, y): negative
/// inside.
fn sd_rrect(px: f32, py: f32, b: [f32; 4], r: f32) -> f32 {
    let (cx, cy) = (b[0] + b[2] * 0.5, b[1] + b[3] * 0.5);
    let dx = abs(px - cx) - (b[2] * 0.5 - r);
    let dy = abs(py - cy) - (b[3] * 0.5 - r);
    let (ox, oy) = (dx.max(0.0), dy.max(0.0));
    dx.max(dy).min(0.0) + sqrt(ox * ox + oy * oy) - r
}

/// The scene's geometry for one window size, in pixels.
struct Scene {
    rrect: [f32; 4],
    grad: [f32; 4],
    pts: [[f32; 2]; 5],
    /// Each segment's length.
    len: [f32; 4],
}

impl Scene {
    fn new(w: u32, h: u32) -> Self {
        let pts = STROKE_PTS.map(|p| point_px(p, w, h));
        let mut len = [0f32; 4];
        for (s, l) in len.iter_mut().enumerate() {
            let (dx, dy) = (pts[s + 1][0] - pts[s][0], pts[s + 1][1] - pts[s][1]);
            *l = sqrt(dx * dx + dy * dy);
        }
        Scene { rrect: box_px(RRECT_BOX, w, h), grad: box_px(GRAD_BOX, w, h), pts, len }
    }

    /// The class of the pixel centre (x, y), canvas coordinates, and the
    /// colour it should hold (for a fringe class, the shape's colour).
    fn at(&self, x: f32, y: f32) -> (Class, [u8; 3]) {
        let d_rrect = sd_rrect(x, y, self.rrect, RRECT_RADIUS);
        let g = self.grad;
        let d_grad = (g[0] - x).max(x - (g[0] + g[2])).max((g[1] - y).max(y - (g[1] + g[3])));
        // Squared distances to the polyline and to its points: they are
        // only compared, so no square root per segment.
        let (mut d2_line, mut d2_join) = (f32::MAX, f32::MAX);
        let (mut core, mut sfringe) = (false, false);
        for s in 0..4 {
            let ([ax, ay], [bx, by]) = (self.pts[s], self.pts[s + 1]);
            let (dx, dy) = (bx - ax, by - ay);
            let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            let (fx, fy) = (ax + t * dx - x, ay + t * dy - y);
            let d2 = fx * fx + fy * fy;
            d2_line = d2_line.min(d2);
            let along = t * self.len[s];
            let clear_of_ends = along >= STROKE_END_GAP && along <= self.len[s] - STROKE_END_GAP;
            core |= clear_of_ends && d2 <= STROKE_CORE * STROKE_CORE;
            sfringe |= clear_of_ends && (SFRINGE_FROM * SFRINGE_FROM..=SFRINGE_TO * SFRINGE_TO).contains(&d2);
        }
        for [px, py] in self.pts {
            d2_join = d2_join.min((x - px) * (x - px) + (y - py) * (y - py));
        }
        let off_line = d2_line > BG_STROKE_GAP * BG_STROKE_GAP;

        if d_rrect <= -FILL_MARGIN && off_line {
            (Class::RRect, RRECT)
        } else if d_grad <= -FILL_MARGIN && off_line {
            (Class::Grad, grad_at(y, g))
        } else if core && d_rrect > FILL_MARGIN && d_grad > FILL_MARGIN {
            (Class::Stroke, STROKE)
        } else if d_rrect >= FILL_MARGIN
            && d_grad >= FILL_MARGIN
            && d2_line >= BG_STROKE_GAP * BG_STROKE_GAP
            && d2_join >= BG_JOIN_GAP * BG_JOIN_GAP
        {
            (Class::Bg, BG)
        } else if sfringe
            && d2_line >= SFRINGE_FROM * SFRINGE_FROM
            && d2_join >= STROKE_END_GAP * STROKE_END_GAP
            && d_rrect > FILL_MARGIN
            && d_grad > FILL_MARGIN
        {
            (Class::StrokeFringe, STROKE)
        } else if abs(d_rrect) <= RFRINGE_BAND && off_line {
            (Class::RRectFringe, RRECT)
        } else {
            (Class::Edge, BG)
        }
    }
}

/// The gradient's colour at canvas row centre `y` in the gradient rect `g`
/// (pixels): mixed linearly from its top edge to its bottom edge, rounded.
fn grad_at(y: f32, g: [f32; 4]) -> [u8; 3] {
    let t = ((y - g[1]) / g[3]).clamp(0.0, 1.0);
    let mut c = [0u8; 3];
    for (k, o) in c.iter_mut().enumerate() {
        let (top, bottom) = (GRAD_TOP[k] as f32, GRAD_BOTTOM[k] as f32);
        *o = (top + t * (bottom - top) + 0.5) as u8;
    }
    c
}

/// The class of pixel (x, y) of a `w` x `h` window, canvas coordinates (y
/// down), and the colour it should hold, R, G, B (for a fringe class, the
/// shape's colour; for an edge pixel, BG).
pub fn class_at(w: u32, h: u32, x: u32, y: u32) -> (Class, [u8; 3]) {
    Scene::new(w, h).at(x as f32 + 0.5, y as f32 + 0.5)
}

/// GL window coordinates (origin bottom left) of the glReadPixels points
/// v3d_gl_femtovg read, for a `w` x `h` window, then the centre (PICTURE's).
pub fn read_points(w: u32, h: u32) -> [(u32, u32); 5] {
    [(1, 1), (w * 26 / 100, h * 75 / 100), (w * 73 / 100, h * 75 / 100), (w * 40 / 100, h * 35 / 100), (w / 2, h / 2)]
}

/// Which part of the check failed first, in v3d_gl_femtovg's order. The
/// value is the APP record's stage byte for a failed check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Failure {
    Counts = 1,
    Bg = 2,
    RRect = 3,
    Grad = 4,
    Stroke = 5,
    StrokeFringe = 6,
    RRectFringe = 7,
    Guard = 8,
    ReadPixels = 9,
}

impl Failure {
    pub fn what(self) -> &'static str {
        match self {
            Failure::Counts => "the pixel classes do not add up",
            Failure::Bg => "background not the clear colour",
            Failure::RRect => "rounded rect colour",
            Failure::Grad => "gradient colour",
            Failure::Stroke => "stroke colour",
            Failure::StrokeFringe => "the stroke's anti-aliased fringe is not drawn partly",
            Failure::RRectFringe => "the rounded rect's anti-aliased fringe is not drawn partly",
            Failure::Guard => "the guard rows after the window changed",
            Failure::ReadPixels => "glReadPixels differs from memory",
        }
    }
}

/// The first wrong pixel: canvas coordinates (y down), what memory holds and
/// what was expected (R, G, B, A; for a fringe pixel, the shape's colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wrong {
    pub class: Class,
    pub x: u32,
    pub y: u32,
    pub got: [u8; 4],
    pub want: [u8; 4],
}

/// Everything the check measured; the counts are over the whole window and
/// indexed as [`CLASSES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub total: u32,
    pub n: [u32; 7],
    pub bad: [u32; 7],
    /// The largest channel difference from the expected colour, or for a
    /// fringe class the largest distance from BG.
    pub worst: [u8; 7],
    pub first_wrong: Option<Wrong>,
    pub guard_bad: u32,
    /// glReadPixels against memory: points read, points different.
    pub reads: u32,
    pub reads_bad: u32,
    /// The first point where glReadPixels differed: GL (x, y) and the GL
    /// value (memory's is `at_gl`).
    pub first_read_bad: Option<(u32, u32, [u8; 4])>,
    /// PICTURE's w1..w3.
    pub picture: [u32; 3],
}

impl Outcome {
    fn get(&self, field: &[u32; 7], c: Class) -> u32 {
        field[c as usize]
    }

    /// The first failure, or None for a pass.
    pub fn verdict(&self) -> Option<Failure> {
        let every_class = self.n.iter().all(|&n| n > 0);
        let b = |c| self.get(&self.bad, c) != 0;
        if self.n.iter().sum::<u32>() != self.total || !every_class {
            Some(Failure::Counts)
        } else if b(Class::Bg) {
            Some(Failure::Bg)
        } else if b(Class::RRect) {
            Some(Failure::RRect)
        } else if b(Class::Grad) {
            Some(Failure::Grad)
        } else if b(Class::Stroke) {
            Some(Failure::Stroke)
        } else if b(Class::StrokeFringe) {
            Some(Failure::StrokeFringe)
        } else if b(Class::RRectFringe) {
            Some(Failure::RRectFringe)
        } else if self.guard_bad != 0 {
            Some(Failure::Guard)
        } else if self.reads_bad != 0 {
            Some(Failure::ReadPixels)
        } else {
            None
        }
    }
}

fn max_diff(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter().zip(b.iter()).map(|(&x, &y)| x.abs_diff(y)).max().unwrap_or(0)
}

fn rgba(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 0xff]
}

/// Whether a fringe pixel is drawn partly: alpha 255, each channel between
/// BG and the shape's colour `want` (within FRINGE_TOL), and more than
/// FRINGE_TOL from BG. Also returns its distance from BG.
fn drawn_partly(got: [u8; 4], want: [u8; 3]) -> (bool, u8) {
    let mut inside = got[3] == 0xff;
    for k in 0..3 {
        let lo = BG[k].min(want[k]).saturating_sub(FRINGE_TOL);
        let hi = BG[k].max(want[k]).saturating_add(FRINGE_TOL);
        inside &= got[k] >= lo && got[k] <= hi;
    }
    let from_bg = max_diff(got, rgba(BG));
    (inside && from_bg > FRINGE_TOL, from_bg)
}

/// Check `frame`. `read_gl(x, y)` is glReadPixels of one RGBA pixel at GL
/// window coordinates (origin bottom left), with the frame's context
/// current.
pub fn check(frame: &Frame, mut read_gl: impl FnMut(u32, u32) -> [u8; 4]) -> Outcome {
    let (w, h) = frame.size();
    let scene = Scene::new(w, h);
    let mut o = Outcome {
        total: 0,
        n: [0; 7],
        bad: [0; 7],
        worst: [0; 7],
        first_wrong: None,
        guard_bad: 0,
        reads: 0,
        reads_bad: 0,
        first_read_bad: None,
        picture: [0; 3],
    };
    for y in 0..h {
        for x in 0..w {
            let (class, want) = scene.at(x as f32 + 0.5, y as f32 + 0.5);
            let got = frame.at_gl(x, h - 1 - y);
            let c = class as usize;
            o.total += 1;
            o.n[c] += 1;
            let wrong = match class {
                Class::Edge => false,
                _ if class.fringe() => {
                    let (ok, from_bg) = drawn_partly(got, want);
                    o.worst[c] = o.worst[c].max(from_bg);
                    !ok
                }
                _ => {
                    let md = max_diff(got, rgba(want));
                    o.worst[c] = o.worst[c].max(md);
                    let tol = if class == Class::Grad { GRAD_TOL } else { EXACT_TOL };
                    md > tol || got[3] != 0xff
                }
            };
            if wrong {
                o.bad[c] += 1;
                if o.first_wrong.is_none() {
                    o.first_wrong = Some(Wrong { class, x, y, got, want: rgba(want) });
                }
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
    for &(x, y) in read_points(w, h).iter() {
        centre = read(x, y, &mut o); /* the last point is the centre */
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
/// verdict, one line per `\n`, each line starting with `femtovg: `.
pub fn report(out: &mut impl core::fmt::Write, frame: &Frame, o: &Outcome) -> core::fmt::Result {
    let (w, h) = frame.size();
    write!(out, "femtovg: window {w}x{h}: {} pixels =", o.total)?;
    for c in CLASSES {
        write!(out, " {} {}", c.name(), o.n[c as usize])?;
    }
    writeln!(out)?;
    for c in CLASSES {
        let i = c as usize;
        if c == Class::Edge {
            continue;
        }
        if c.fringe() {
            writeln!(
                out,
                "femtovg:   {} not drawn partly {} of {} (largest distance from bg {})",
                c.name(),
                o.bad[i],
                o.n[i],
                o.worst[i]
            )?;
        } else {
            writeln!(out, "femtovg:   {} wrong {} (largest channel diff {})", c.name(), o.bad[i], o.worst[i])?;
        }
    }
    if let Some(f) = o.first_wrong {
        writeln!(
            out,
            "femtovg:   first wrong ({}) at canvas {},{}: 0x{:08x}, expected 0x{:08x} (R in the low byte)",
            f.class.name(),
            f.x,
            f.y,
            word(f.got),
            word(f.want)
        )?;
    }
    writeln!(out, "femtovg:   guard rows: {} of {} pixels changed", o.guard_bad, frame.guard().len())?;
    writeln!(out, "femtovg:   glReadPixels against memory: {} of {} points differ", o.reads_bad, o.reads)?;
    if let Some((x, y, gl)) = o.first_read_bad {
        writeln!(
            out,
            "femtovg:   first at GL {x},{y}: glReadPixels 0x{:08x}, memory 0x{:08x}",
            word(gl),
            word(frame.at_gl(x, y))
        )?;
    }
    let [p1, p2, p3] = o.picture;
    writeln!(
        out,
        "femtovg: picture: centre 0x{p1:08x}, column crc 0x{p2:08x} ({PICTURE_SAMPLES} samples), size 0x{p3:08x}"
    )?;
    match o.verdict() {
        None => writeln!(out, "femtovg: check PASS (every pixel read back)"),
        Some(f) => writeln!(out, "femtovg: check FAIL: {} (code {})", f.what(), f as u8),
    }
}

#[cfg(test)]
mod tests;
