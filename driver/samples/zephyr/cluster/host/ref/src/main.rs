// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host reference for the cluster sample: the same UI, state and clock as the
//! board image, drawn by Slint's software renderer, so the board's frame has
//! something to be compared with.
//!
//! What is the same as on the board: ../../ui/zephyr-cluster.slint (the
//! cluster's AppWindow, re-exported), the cluster's application layer
//! (../../rust/src/cluster.rs, compiled here too), the frame sequence and the
//! animation clock (main.c: frame 1 at 0 ms, then FRAME_TICKS telemetry
//! periods, frame 2 at 1000 ms, frame 3 at 3000 ms), and the window size.
//! What differs: the renderer (software here, FemtoVG on the V3D) and the
//! glyphs (rasterised at run time from the same Noto Sans files here,
//! pre-rendered bitmaps on the board). Compare where neither matters: flat
//! areas, not text edges (`probes` below).
//!
//! Usage: cluster_ref <width> <height> <out.rgba>
//! Writes width*height*4 bytes (R, G, B, 255; rows top down) and prints the
//! probe table main.c carries.
extern crate alloc;

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Rgb8Pixel};

slint::include_modules!();

#[path = "../../../rust/src/cluster.rs"]
mod cluster;

// The checked frame's sequence: the one definition, the cluster sample's
// check crate (../../check), which the image's C side reads too.
use cluster_check::{FRAME2_MS, FRAME3_MS, FRAME_TICKS};

thread_local! {
    static CLOCK_MS: Cell<u64> = const { Cell::new(0) };
}

struct RefPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for RefPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> core::time::Duration {
        core::time::Duration::from_millis(CLOCK_MS.with(|c| c.get()))
    }
}

fn frame(window: &MinimalSoftwareWindow, buf: &mut [Rgb8Pixel], w: usize, clock_ms: u64) {
    CLOCK_MS.with(|c| c.set(clock_ms));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(buf, w);
    });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: {} <width> <height> <out.rgba>", args[0]);
        std::process::exit(2);
    }
    let w: usize = args[1].parse().expect("width");
    let h: usize = args[2].parse().expect("height");

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(RefPlatform { window: window.clone() })).unwrap();
    // Slint's software renderer ignores transform-scale (internal/renderers/
    // software/lib.rs, `fn scale`: "TODO"), which AppWindow uses to fit the
    // design to the window: at any scale but 1 it drew the design unscaled
    // and cropped (the 1024x600 reference of 2026-10-03, which the AM62P's
    // frame missed at every probe). The window's scale factor does the same
    // scaling, and the software renderer honours it: with scale factor s
    // the logical window is w/s x h/s, for which AppWindow's own scale is
    // exactly 1, so it applies no transform; the picture is the board's.
    let s = window_scale(w as f32, h as f32);
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: s });
    window.set_size(PhysicalSize::new(w as u32, h as u32));
    println!("window {w}x{h}: AppWindow's scale {s:.4}, applied as the window's scale factor");
    let ui = AppWindow::new().expect("component");
    ui.show().expect("show");

    let mut buf = vec![Rgb8Pixel::new(0, 0, 0); w * h];
    let mut drive = cluster::Drive::new();
    frame(&window, &mut buf, w, 0);
    drive.advance(&ui, FRAME_TICKS);
    frame(&window, &mut buf, w, FRAME2_MS);
    frame(&window, &mut buf, w, FRAME3_MS);
    let speed = drive.last.as_ref().map_or(-1.0, |t| t.speed_kmh);
    println!("state: {} ticks, speed {:.1} km/h (x10 {})", drive.ticks, speed, (speed * 10.0) as i32);

    let mut out = Vec::with_capacity(w * h * 4);
    for p in &buf {
        out.extend_from_slice(&[p.r, p.g, p.b, 255]);
    }
    std::fs::write(&args[3], &out).expect("write");
    println!("wrote {} ({}x{}, {} bytes)", args[3], w, h, out.len());
    // the blind spot, for compare.py: one band a line, design coordinates
    let bands = gradient_strokes(&ui);
    let mut blind = format!("# cx cy r_in r_out a0 a1 margin (host/ref/src/main.rs gradient_strokes)\n");
    for b in &bands {
        blind += &format!("{} {} {} {} {} {} {}\n", b.cx, b.cy, b.r_in, b.r_out, b.a0, b.a1, BAND_MARGIN);
    }
    let blind_path = args[3].replace(".rgba", ".blind.txt");
    std::fs::write(&blind_path, blind).expect("write");
    println!(
        "wrote {blind_path}: charge arc {:.1}..{:.1} deg, speed arc {:.1}..{:.1} deg",
        bands[0].a0, bands[0].a1, bands[1].a0, bands[1].a1
    );
    probes(&buf, &bands, w, h);
}

/// An arc band in the design's coordinates (app-window.slint, theme.slint:
/// 1920x720, y down) that a gradient-stroked Path covers: radii r_in..r_out
/// around (cx, cy), angles a0..a1 in degrees clockwise from 12 o'clock (the
/// cluster's own convention, gauge-parts.slint).
#[derive(Clone, Copy)]
struct Band {
    cx: f32,
    cy: f32,
    r_in: f32,
    r_out: f32,
    a0: f32,
    a1: f32,
}

/// The host reference's blind spot: Slint's software renderer strokes a Path
/// with one colour, the brush's `color()` (internal/renderers/software/
/// lib.rs:3186; for a gradient, a single stop's colour), so wherever a
/// gradient-stroked Path draws, the reference is not the picture. On the
/// board (CLUSTER1, 61359eba) the speed arc matched the design's conic
/// gradient within 3-9 at four angles while the reference read white. The
/// probes skip these bands, run.sh writes them next to the reference for
/// compare.py, and host/check_ui.py fails the build if the UI gains another
/// gradient stroke. The angles are the frame's own (read from the UI after
/// frame 3, when the animations have settled), so the solid-coloured dial
/// tracks at the same radii stay probed.
fn gradient_strokes(ui: &AppWindow) -> [Band; 2] {
    // app-window.slint:212-254: 5 ArcStrokes, r = 200.13 + 1.99 k (k 0..4),
    // 4.5 px wide, from min(0, charge) to max(0, charge)
    let c = ui.get_charge_angle();
    // app-window.slint:92-97, 296-315: r 204, 12.66 px wide, from dial-a0 to
    // speed-angle
    let speed_angle = -145.0 + 290.0 * ui.get_speed().min(240.0) / 240.0;
    [
        Band { cx: 322.0, cy: 325.0, r_in: 200.13 - 2.25, r_out: 208.09 + 2.25, a0: c.min(0.0), a1: c.max(0.0) },
        Band { cx: 1598.0, cy: 325.0, r_in: 204.0 - 6.33, r_out: 204.0 + 6.33, a0: -145.0, a1: speed_angle },
    ]
}

/// Design pixels added on every side of a band: antialiasing, the round of
/// the stroke's ends, and the 3 px a probe's 7x7 flatness box reaches.
const BAND_MARGIN: f32 = 8.0;

/// AppWindow's scale for a w x h window: it keeps fit-width =
/// max(1605, min(1920, 720 * w / h)) of the 1920-wide design, centred, and
/// scales it by min(w / fit-width, h / 720) about the window's centre
/// (app-window.slint:110-129). 1 at 1920x720, 0.797 at 1280x800, 0.638 at
/// 1024x600.
fn window_scale(w: f32, h: f32) -> f32 {
    let fit = (720.0 * w / h).min(1920.0).max(1605.0);
    (w / fit).min(h / 720.0)
}

/// Window pixel (x, y) to the design's coordinates (window_scale).
fn to_design(x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
    let s = window_scale(w, h);
    (960.0 + (x - w / 2.0) / s, 360.0 + (y - h / 2.0) / s)
}

fn in_band(b: &Band, dx: f32, dy: f32) -> bool {
    let (vx, vy) = (dx - b.cx, dy - b.cy);
    let r = (vx * vx + vy * vy).sqrt();
    if r < b.r_in - BAND_MARGIN || r > b.r_out + BAND_MARGIN {
        return false;
    }
    // clockwise from 12 o'clock, as the cluster measures its dials
    let a = vx.atan2(-vy).to_degrees();
    let pad = (BAND_MARGIN / r.max(1.0)).to_degrees();
    a >= b.a0 - pad && a <= b.a1 + pad
}

fn in_blind_spot(bands: &[Band], x: usize, y: usize, w: usize, h: usize) -> bool {
    let (dx, dy) = to_design(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32);
    bands.iter().any(|b| in_band(b, dx, dy))
}

/// Points where the frame is flat over a 7x7 box (every channel within 3 of
/// the centre), not near black (R+G+B at least 60) and outside the blind spot
/// (gradient_strokes), one per cell of a 12 x 5 grid: the flattest, then the
/// brightest, in each cell. Flat areas are where the two renderers must
/// agree; edges and glyphs are where they may not, and a black point says
/// little. Printed as main.c's table.
fn probes(buf: &[Rgb8Pixel], bands: &[Band], w: usize, h: usize) {
    let (gx, gy) = (12usize, 5usize);
    let at = |x: usize, y: usize| buf[y * w + x];
    let skipped =
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| in_blind_spot(bands, x, y, w, h)).count();
    println!("/* host reference {w}x{h}: x, y, r, g, b (flat over 7x7 within 3, R+G+B >= 60, outside the");
    println!(" * gradient strokes: {skipped} pixels skipped) */");
    for cy in 0..gy {
        for cx in 0..gx {
            let (x0, x1) = (cx * w / gx + 3, (cx + 1) * w / gx - 4);
            let (y0, y1) = (cy * h / gy + 3, (cy + 1) * h / gy - 4);
            let mut best: Option<(u32, usize, usize)> = None;
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let c = at(x, y);
                    let lum = c.r as u32 + c.g as u32 + c.b as u32;
                    if lum >= 60 && !in_blind_spot(bands, x, y, w, h) {
                        let mut spread = 0u32;
                        for dy in 0..7 {
                            for dx in 0..7 {
                                let p = at(x + dx - 3, y + dy - 3);
                                for (a, b) in [(p.r, c.r), (p.g, c.g), (p.b, c.b)] {
                                    spread = spread.max((a as i32 - b as i32).unsigned_abs());
                                }
                            }
                        }
                        let score = spread * 1000 + (765 - lum);
                        if spread <= 3 && best.is_none_or(|b| score < b.0) {
                            best = Some((score, x, y));
                        }
                    }
                    x += 2;
                }
                y += 2;
            }
            if let Some((_, x, y)) = best {
                let c = at(x, y);
                println!("\t{{ {x}, {y}, {{ {}, {}, {} }} }},", c.r, c.g, c.b);
            }
        }
    }
}
