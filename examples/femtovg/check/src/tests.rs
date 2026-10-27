// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the femtovg check on synthetic frames.
use super::*;

const W: u32 = WIDTH;
const H: u32 = HEIGHT;

/// A memory word for R, G, B, A in `format`.
fn mem_word(format: u32, c: [u8; 4]) -> u32 {
    if format & !FORMAT_BOTTOM_UP == FORMAT_XRGB8888 {
        u32::from_le_bytes([c[2], c[1], c[0], c[3]])
    } else {
        u32::from_le_bytes(c)
    }
}

/// The memory index of canvas pixel (x, y) (y down).
fn mem_index(format: u32, x: u32, y: u32) -> usize {
    let row = if format & FORMAT_BOTTOM_UP != 0 { H - 1 - y } else { y };
    (row * W + x) as usize
}

/// What a correct GL leaves at a pixel of `class` that should hold `want`:
/// the colour itself, a fringe half way between BG and the colour, an edge
/// pixel BG.
fn painted(class: Class, want: [u8; 3]) -> [u8; 4] {
    match class {
        Class::StrokeFringe | Class::RRectFringe => {
            let mut c = [0u8; 4];
            for k in 0..3 {
                c[k] = ((BG[k] as u32 + want[k] as u32) / 2) as u8;
            }
            c[3] = 0xff;
            c
        }
        _ => rgba(want),
    }
}

/// A correctly drawn frame in `format`, the guard rows intact.
fn drawn(format: u32) -> Vec<u32> {
    let scene = Scene::new(W, H);
    let mut px = vec![GUARD; (W * (H + GUARD_ROWS)) as usize];
    for y in 0..H {
        for x in 0..W {
            let (class, want) = scene.at(x as f32 + 0.5, y as f32 + 0.5);
            px[mem_index(format, x, y)] = mem_word(format, painted(class, want));
        }
    }
    px
}

/// glReadPixels as a correct GL answers it for `px`.
fn gl_reader(px: &[u32], format: u32) -> impl FnMut(u32, u32) -> [u8; 4] + '_ {
    move |x, y| Frame::new(px, W, H, GUARD_ROWS, format).unwrap().at_gl(x, y)
}

fn run(px: &[u32], format: u32) -> Outcome {
    let f = Frame::new(px, W, H, GUARD_ROWS, format).unwrap();
    check(&f, gl_reader(px, format))
}

/// The first pixel of `class` in canvas row order.
fn first_of(class: Class) -> (u32, u32, [u8; 3]) {
    let scene = Scene::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let (c, want) = scene.at(x as f32 + 0.5, y as f32 + 0.5);
            if c == class {
                return (x, y, want);
            }
        }
    }
    panic!("no {} pixel", class.name());
}

const FORMATS: [u32; 4] =
    [FORMAT_RGBA8888, FORMAT_XRGB8888, FORMAT_RGBA8888 | FORMAT_BOTTOM_UP, FORMAT_XRGB8888 | FORMAT_BOTTOM_UP];

#[test]
fn sqrt_is_stds() {
    let mut v = 1e-6f32;
    while v < 1e7 {
        assert_eq!(sqrt(v), v.sqrt(), "{v}");
        v *= 1.013;
    }
    assert_eq!(sqrt(0.0), 0.0);
    for i in 0..100_000u32 {
        let v = i as f32 * 0.37;
        assert_eq!(sqrt(v), v.sqrt(), "{v}");
    }
}

#[test]
fn sd_rrect_is_femtovgs() {
    let b = [100.0, 100.0, 200.0, 100.0];
    // The centre, an edge midpoint, a corner's arc and a point beyond it.
    assert_eq!(sd_rrect(200.0, 150.0, b, 16.0), -50.0);
    assert_eq!(sd_rrect(200.0, 100.0, b, 16.0), 0.0);
    let (cx, cy) = (116.0, 116.0); /* the top left corner's centre */
    let s = 16.0 / 2f32.sqrt();
    assert!(abs(sd_rrect(cx - s, cy - s, b, 16.0)) < 1e-4);
    assert!((sd_rrect(100.0, 100.0, b, 16.0) - (16.0 * 2f32.sqrt() - 16.0)).abs() < 1e-4);
}

#[test]
fn a_correct_frame_passes_in_every_format() {
    for format in FORMATS {
        let o = run(&drawn(format), format);
        assert_eq!(o.verdict(), None, "format {format:#x}: {o:?}");
        assert_eq!(o.total, W * H);
        assert_eq!(o.n.iter().sum::<u32>(), W * H);
        for c in CLASSES {
            assert!(o.n[c as usize] > 0, "no {} pixel", c.name());
        }
        assert!(o.n[Class::Bg as usize] > 200_000 && o.n[Class::RRect as usize] > 50_000, "{o:?}");
        assert_eq!((o.reads, o.reads_bad, o.guard_bad), (5 + PICTURE_SAMPLES as u32, 0, 0));
    }
}

/// The class counts of the 1024x600 window, so a change of the geometry or
/// the margins shows as a diff. They are the counts v3d_gl_femtovg's C
/// classifier (check_window) gave for the same window, compiled on the
/// host: the port classifies the same pixels.
#[test]
fn class_counts() {
    let o = run(&drawn(FORMAT_RGBA8888), FORMAT_RGBA8888);
    let n = |c: Class| o.n[c as usize];
    let got: Vec<(&str, u32)> = CLASSES.iter().map(|&c| (c.name(), n(c))).collect();
    assert_eq!(
        got,
        [
            ("rrect", 64_060),
            ("grad", 67_760),
            ("stroke", 2_668),
            ("bg", 455_186),
            ("sfringe", 1_062),
            ("rfringe", 204),
            ("edge", 23_460),
        ],
        "{got:?}"
    );
}

#[test]
fn the_gradient_runs_top_to_bottom() {
    let g = box_px(GRAD_BOX, W, H);
    assert_eq!(grad_at(g[1], g), GRAD_TOP);
    assert_eq!(grad_at(g[1] + g[3], g), GRAD_BOTTOM);
    assert_eq!(grad_at(0.0, g), GRAD_TOP);
    assert_eq!(grad_at(H as f32, g), GRAD_BOTTOM);
}

/// One wrong pixel of each class fails with that class's code, and the
/// report names it.
#[test]
fn a_planted_wrong_pixel_fails_by_class() {
    let cases = [
        (Class::Bg, Failure::Bg),
        (Class::RRect, Failure::RRect),
        (Class::Grad, Failure::Grad),
        (Class::Stroke, Failure::Stroke),
        (Class::StrokeFringe, Failure::StrokeFringe),
        (Class::RRectFringe, Failure::RRectFringe),
    ];
    for (class, failure) in cases {
        let mut px = drawn(FORMAT_RGBA8888);
        let (x, y, want) = first_of(class);
        // A fringe left at BG (not drawn), a solid pixel off by 9.
        let bad = if class.fringe() { rgba(BG) } else { [want[0].wrapping_add(9), want[1], want[2], 0xff] };
        px[mem_index(FORMAT_RGBA8888, x, y)] = mem_word(FORMAT_RGBA8888, bad);
        let o = run(&px, FORMAT_RGBA8888);
        assert_eq!(o.verdict(), Some(failure), "{}", class.name());
        assert_eq!(o.bad[class as usize], 1);
        let f = o.first_wrong.unwrap();
        assert_eq!((f.class, f.x, f.y, f.got), (class, x, y, bad));
    }
}

/// An edge pixel is not checked.
#[test]
fn edge_pixels_are_counted_only() {
    let mut px = drawn(FORMAT_RGBA8888);
    let (x, y, _) = first_of(Class::Edge);
    px[mem_index(FORMAT_RGBA8888, x, y)] = SENTINEL;
    assert_eq!(run(&px, FORMAT_RGBA8888).verdict(), None);
}

#[test]
fn tolerances() {
    for (class, tol) in [(Class::Bg, EXACT_TOL), (Class::RRect, EXACT_TOL), (Class::Grad, GRAD_TOL)] {
        let (x, y, want) = first_of(class);
        for (d, fails) in [(tol, false), (tol + 1, true)] {
            let mut px = drawn(FORMAT_RGBA8888);
            px[mem_index(FORMAT_RGBA8888, x, y)] = mem_word(FORMAT_RGBA8888, [want[0], want[1] + d, want[2], 0xff]);
            let o = run(&px, FORMAT_RGBA8888);
            assert_eq!(o.verdict().is_some(), fails, "{} off by {d}", class.name());
        }
        let mut px = drawn(FORMAT_RGBA8888);
        px[mem_index(FORMAT_RGBA8888, x, y)] = mem_word(FORMAT_RGBA8888, [want[0], want[1], want[2], 0xfe]);
        assert!(run(&px, FORMAT_RGBA8888).verdict().is_some(), "{} alpha 254", class.name());
    }
}

/// A fringe drawn fully is drawn partly enough (between BG and the colour,
/// away from BG); one beyond the colour or just off BG is not.
#[test]
fn a_fringe_is_between_bg_and_the_colour() {
    let (x, y, want) = first_of(Class::StrokeFringe);
    let i = mem_index(FORMAT_RGBA8888, x, y);
    for (c, ok) in [
        (rgba(want), true),
        ([BG[0] + FRINGE_TOL + 1, BG[1] + FRINGE_TOL + 1, BG[2], 0xff], true),
        ([BG[0] + FRINGE_TOL, BG[1] + FRINGE_TOL, BG[2], 0xff], false),
        ([want[0], want[1], want[2] + FRINGE_TOL + 1, 0xff], false),
    ] {
        let mut px = drawn(FORMAT_RGBA8888);
        px[i] = mem_word(FORMAT_RGBA8888, c);
        assert_eq!(run(&px, FORMAT_RGBA8888).verdict().is_none(), ok, "{c:?}");
    }
}

#[test]
fn guard_and_readpixels_each_fail() {
    let mut px = drawn(FORMAT_RGBA8888);
    let last = px.len() - 1;
    px[last] = SENTINEL;
    assert_eq!(run(&px, FORMAT_RGBA8888).verdict(), Some(Failure::Guard));

    let px = drawn(FORMAT_RGBA8888);
    let f = Frame::new(&px, W, H, GUARD_ROWS, FORMAT_RGBA8888).unwrap();
    let o = check(&f, |x, y| if (x, y) == (1, 1) { [0; 4] } else { f.at_gl(x, y) });
    assert_eq!(o.verdict(), Some(Failure::ReadPixels));
    assert_eq!((o.reads_bad, o.first_read_bad), (1, Some((1, 1, [0; 4]))));
}

/// A frame drawn upside down fails (the window's orientation is checked).
#[test]
fn orientation_is_checked() {
    let px = drawn(FORMAT_RGBA8888);
    let o = run(&px, FORMAT_RGBA8888 | FORMAT_BOTTOM_UP);
    assert!(o.verdict().is_some());
}

#[test]
fn an_untouched_window_fails_on_bg() {
    let mut px = vec![SENTINEL; (W * H) as usize];
    px.extend(std::iter::repeat_n(GUARD, (W * GUARD_ROWS) as usize));
    assert_eq!(run(&px, FORMAT_RGBA8888).verdict(), Some(Failure::Bg));
}

#[test]
fn picture_is_the_centre_and_the_column() {
    let px = drawn(FORMAT_RGBA8888);
    let o = run(&px, FORMAT_RGBA8888);
    let f = Frame::new(&px, W, H, GUARD_ROWS, FORMAT_RGBA8888).unwrap();
    let mut column = [[0u8; 4]; PICTURE_SAMPLES];
    for (c, &(x, y)) in column.iter_mut().zip(picture_column(W, H).iter()) {
        *c = f.at_gl(x, y);
    }
    assert_eq!(o.picture, picture_words(f.at_gl(W / 2, H / 2), &column, W, H));
    assert_eq!(o.picture[0], u32::from_le_bytes(rgba(BG)), "the centre is the background");
    assert_eq!(o.picture[2], record::size_word(W, H));
}

#[test]
fn failure_codes() {
    let all = [
        Failure::Counts,
        Failure::Bg,
        Failure::RRect,
        Failure::Grad,
        Failure::Stroke,
        Failure::StrokeFringe,
        Failure::RRectFringe,
        Failure::Guard,
        Failure::ReadPixels,
    ];
    for (i, f) in all.iter().enumerate() {
        assert_eq!(*f as u8, i as u8 + 1);
        assert!(!f.what().is_empty());
    }
}

#[test]
fn the_report_reads_as_the_board_console() {
    let px = drawn(FORMAT_RGBA8888);
    let f = Frame::new(&px, W, H, GUARD_ROWS, FORMAT_RGBA8888).unwrap();
    let o = check(&f, |x, y| f.at_gl(x, y));
    let mut s = String::new();
    report(&mut s, &f, &o).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert!(lines.iter().all(|l| l.starts_with("femtovg: ")), "{s}");
    assert!(lines[0].starts_with("femtovg: window 1024x600: 614400 pixels = rrect "), "{s}");
    assert_eq!(*lines.last().unwrap(), "femtovg: check PASS (every pixel read back)");
    assert!(s.contains("femtovg:   sfringe not drawn partly 0 of "), "{s}");
}
