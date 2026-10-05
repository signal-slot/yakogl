// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the triangle check on synthetic frames.
use super::*;

const W: u32 = WIDTH;
const H: u32 = HEIGHT;

/// A memory word for R, G, B, A in `format`.
fn word(format: u32, c: [u8; 4]) -> u32 {
    if format & !FORMAT_BOTTOM_UP == FORMAT_XRGB8888 {
        u32::from_le_bytes([c[2], c[1], c[0], c[3]])
    } else {
        u32::from_le_bytes(c)
    }
}

fn mem_index(format: u32, x: u32, gy: u32) -> usize {
    let row = if format & FORMAT_BOTTOM_UP != 0 { gy } else { H - 1 - gy };
    (row * W + x) as usize
}

/// A correctly drawn frame in `format`: the reference everywhere (the edge
/// band drawn), the guard rows intact.
fn drawn(format: u32) -> Vec<u32> {
    let mut px = vec![0u32; (W * (H + GUARD_ROWS)) as usize];
    for gy in 0..H {
        for x in 0..W {
            px[mem_index(format, x, gy)] = word(format, reference_at(W, H, x, gy));
        }
    }
    for p in px[(W * H) as usize..].iter_mut() {
        *p = GUARD;
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

/// The first edge pixel in GL row order.
fn first_edge_pixel() -> (u32, u32) {
    let t = Tri::new(W, H);
    for gy in 0..H {
        for x in 0..W {
            let mut l = [0f32; 3];
            if t.at(x as f32 + 0.5, gy as f32 + 0.5, &mut l) == Class::Edge {
                return (x, gy);
            }
        }
    }
    panic!("no edge pixel");
}

const FORMATS: [u32; 4] =
    [FORMAT_RGBA8888, FORMAT_XRGB8888, FORMAT_RGBA8888 | FORMAT_BOTTOM_UP, FORMAT_XRGB8888 | FORMAT_BOTTOM_UP];

#[test]
fn a_correct_frame_passes_in_every_format() {
    for format in FORMATS {
        let o = run(&drawn(format), format);
        assert_eq!(o.verdict(), None, "format {format:#x}: {o:?}");
        assert_eq!(o.total, W * H);
        assert_eq!(o.n_in + o.n_out + o.n_edge, W * H);
        assert!(o.n_in > 100_000 && o.n_out > 100_000 && o.n_edge > 1000, "{o:?}");
        assert_eq!(o.edge_drawn, o.n_edge);
        assert_eq!((o.worst_in, o.reads, o.reads_bad, o.guard_bad), (0, 4 + 19, 0, 0));
    }
}

/// The edge band accepts the clear colour too (a rasteriser that does not
/// cover a pixel the reference half-covers).
#[test]
fn edge_pixels_may_be_the_clear_colour() {
    let f = FORMAT_RGBA8888;
    let t = Tri::new(W, H);
    let mut px = drawn(f);
    let mut cleared = 0;
    for gy in 0..H {
        for x in 0..W {
            let mut l = [0f32; 3];
            if t.at(x as f32 + 0.5, gy as f32 + 0.5, &mut l) == Class::Edge {
                px[mem_index(f, x, gy)] = word(f, CLEAR_RGBA);
                cleared += 1;
            }
        }
    }
    let o = run(&px, f);
    assert_eq!(o.verdict(), None, "{o:?}");
    assert_eq!((o.n_edge, o.edge_drawn), (cleared, 0));
}

/// The tolerance, at the centre pixel, on every channel, in both
/// directions: 3 away passes, 4 away fails and names the pixel.
#[test]
fn tolerance_is_three_per_channel() {
    let f = FORMAT_XRGB8888;
    let (x, y) = (W / 2, H / 2);
    let base = reference_at(W, H, x, y);
    // the centre's channels are all in 4..=251, so +-4 never wraps
    assert!(base[..3].iter().all(|&c| (4..=251).contains(&c)), "{base:?}");
    let mut cases = 0;
    for channel in 0..3 {
        for (delta, pass) in [(3i16, true), (-3, true), (4, false), (-4, false)] {
            let mut px = drawn(f);
            let mut c = base;
            c[channel] = (c[channel] as i16 + delta) as u8;
            px[mem_index(f, x, y)] = word(f, c);
            let o = run(&px, f);
            let want = if pass { None } else { Some(Failure::Inside) };
            assert_eq!(o.verdict(), want, "channel {channel} delta {delta}");
            assert_eq!(o.worst_in, delta.unsigned_abs() as u8);
            assert_eq!(o.bad_in, (!pass) as u32);
            if !pass {
                assert_eq!(o.first_wrong, Some(Wrong { x, y, got: c, want: base }));
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 12);
}

/// One planted wrong pixel fails the check, and the first wrong pixel is
/// reported where it was planted.
#[test]
fn a_planted_wrong_pixel_fails() {
    let f = FORMAT_RGBA8888 | FORMAT_BOTTOM_UP;
    // outside: near the bottom-left corner, one LSB off the clear colour
    let mut px = drawn(f);
    px[mem_index(f, 1, 1)] = word(f, [0x21, 0x20, 0x20, 0xff]);
    let o = run(&px, f);
    assert_eq!((o.verdict(), o.bad_out), (Some(Failure::Outside), 1));
    assert_eq!(o.first_wrong.map(|w| (w.x, w.y)), Some((1, 1)));
    assert_eq!(o.reads_bad, 0, "the reader reads the same memory");

    // inside: the sentinel left near the centroid (nothing drew there)
    let mut px = drawn(f);
    let (cx, cy) = (W / 2, H * 37 / 100);
    px[mem_index(f, cx, cy)] = SENTINEL;
    let o = run(&px, f);
    assert_eq!((o.verdict(), o.bad_in), (Some(Failure::Inside), 1));
    assert_eq!(o.first_wrong.map(|w| (w.x, w.y)), Some((cx, cy)));

    // inside: alpha not 255
    let mut px = drawn(f);
    let mut c = reference_at(W, H, cx, cy);
    c[3] = 0xfe;
    px[mem_index(f, cx, cy)] = word(f, c);
    let o = run(&px, f);
    assert_eq!((o.verdict(), o.bad_in, o.worst_in), (Some(Failure::Inside), 1, 1));
}

#[test]
fn edge_guard_and_readpixels_each_fail() {
    let f = FORMAT_RGBA8888;
    // an edge pixel that is neither the clear colour nor the gradient
    let mut px = drawn(f);
    let (ex, ey) = first_edge_pixel();
    px[mem_index(f, ex, ey)] = SENTINEL;
    let o = run(&px, f);
    assert_eq!((o.verdict(), o.bad_edge), (Some(Failure::Edge), 1));
    assert_eq!(o.first_wrong.map(|w| (w.x, w.y)), Some((ex, ey)));

    let mut px = drawn(f);
    let last = px.len() - 1;
    px[last] = 0;
    let o = run(&px, f);
    assert_eq!((o.verdict(), o.guard_bad), (Some(Failure::Guard), 1));

    let px = drawn(f);
    let fr = Frame::new(&px, W, H, GUARD_ROWS, f).unwrap();
    let o = check(&fr, |_, _| [0, 0, 0, 0xff]);
    assert_eq!((o.verdict(), o.reads, o.reads_bad), (Some(Failure::ReadPixels), 23, 23));
}

/// A GL that reads the rows the wrong way up, or a window whose orientation
/// flag is wrong, is caught.
#[test]
fn orientation_is_checked() {
    let f = FORMAT_RGBA8888;
    let px = drawn(f);
    let fr = Frame::new(&px, W, H, GUARD_ROWS, f).unwrap();
    let o = check(&fr, |x, y| fr.at_gl(x, H - 1 - y));
    assert_eq!(o.verdict(), Some(Failure::ReadPixels));

    // top-down memory declared bottom-up: the picture is upside down
    let o = run(&px, f | FORMAT_BOTTOM_UP);
    assert_eq!(o.verdict(), Some(Failure::Inside), "{o:?}");
}

/// A frame with no outside pixel cannot pass, whatever its pixels.
#[test]
fn counts_need_both_classes() {
    let o = Outcome { n_out: 0, ..run(&drawn(FORMAT_RGBA8888), FORMAT_RGBA8888) };
    assert_eq!(o.verdict(), Some(Failure::Counts));
}

/// PICTURE is gc_gl_triangle's packing: w1 the centre's glReadPixels bytes
/// little-endian, w2 the CRC-32/IEEE of the 19 column samples' bytes in
/// ascending y, w3 W << 16 | H.
#[test]
fn picture_packing_is_gc_gl_triangles() {
    let col = picture_column(W, H);
    assert_eq!(col[0], (512, 75));
    assert_eq!(col[PICTURE_SAMPLES - 1], (512, 525));
    assert!(col.windows(2).all(|p| p[1].1 - p[0].1 == 25));
    let mut column = [[0u8; 4]; PICTURE_SAMPLES];
    for (k, c) in column.iter_mut().enumerate() {
        *c = [k as u8, 2 * k as u8, 3 * k as u8, 0xff];
    }
    let words = picture_words([0x80, 0x3f, 0x40, 0xff], &column, W, H);
    let bytes: Vec<u8> = column.iter().flatten().copied().collect();
    assert_eq!(words, [0xff40_3f80, record::crc32_ieee_update(0, &bytes), 0x0400_0258]);
    // the check reads the centre at (W/2, H/2) and the column, in order
    let px = drawn(FORMAT_XRGB8888);
    let o = run(&px, FORMAT_XRGB8888);
    let fr = Frame::new(&px, W, H, GUARD_ROWS, FORMAT_XRGB8888).unwrap();
    let mut want_col = [[0u8; 4]; PICTURE_SAMPLES];
    for (c, &(x, y)) in want_col.iter_mut().zip(col.iter()) {
        *c = fr.at_gl(x, y);
    }
    assert_eq!(o.picture, picture_words(fr.at_gl(W / 2, H / 2), &want_col, W, H));
}

/// The i.MX8MP baseline PICTURE (record.rs,
/// writer_reproduces_the_baseline_picture: gc_gl_triangle at 1024x600,
/// kexec and netboot, 2026-10-03) is exactly this reference's PICTURE: the
/// GC7000 drew the ideal interpolated colour at the centre and at all 19
/// column samples, and this crate's coordinates and packing are
/// gc_gl_triangle's. The read points but (1, 1) and the column samples are
/// all inside the triangle, so the words do not depend on the clear colour
/// (which this sample changed from gc_gl_triangle's blue).
#[test]
fn the_reference_picture_is_the_gc7000_baseline() {
    for format in FORMATS {
        assert_eq!(run(&drawn(format), format).picture, [0xff40_3f80, 0x3570_6c4e, 0x0400_0258], "{format:#x}");
    }
    let t = Tri::new(W, H);
    let mut points = read_points(W, H).to_vec();
    points.extend(picture_column(W, H));
    for (x, y) in points.into_iter().skip(1) {
        let mut l = [0f32; 3];
        assert_eq!(t.at(x as f32 + 0.5, y as f32 + 0.5, &mut l), Class::Inside, "({x},{y})");
    }
}

#[test]
fn frame_refusals() {
    let px = vec![0u32; (W * (H + GUARD_ROWS)) as usize];
    assert_eq!(Frame::new(&px, W, H, GUARD_ROWS, 3).err(), Some(FrameError::Format));
    assert_eq!(Frame::new(&px[1..], W, H, GUARD_ROWS, 1).err(), Some(FrameError::Short));
    assert_eq!(Frame::new(&px, 1, H, 0, 1).err(), Some(FrameError::Size));
    assert_eq!(Frame::new(&px, W, 0, 0, 1).err(), Some(FrameError::Size));
}

/// The failure codes are the APP record's stage byte: distinct, 1..=6.
#[test]
fn failure_codes() {
    let all = [Failure::Counts, Failure::Inside, Failure::Outside, Failure::Edge, Failure::Guard, Failure::ReadPixels];
    for (i, f) in all.iter().enumerate() {
        assert_eq!(*f as u8, i as u8 + 1);
    }
}

// ---- the shader texts are the offline tables' -------------------------------

/// SHA-256 (FIPS 180-4), for the test below only.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hs: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = hs;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (h, x) in hs.iter_mut().zip(v.iter()) {
            *h = h.wrapping_add(*x);
        }
    }
    let mut out = [0u8; 32];
    for (i, h) in hs.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&h.to_be_bytes());
    }
    out
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn sha256_known_answers() {
    assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    // two blocks (the values are `sha256sum`'s)
    assert_eq!(
        hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

/// The texts are GLSL ES 1.00: `#version 100` on the first line, ES 1.00
/// storage qualifiers, gl_FragColor; none of the desktop/Vulkan forms the
/// `#version 450` files had. (Whether our compiler BUILDS them is the online
/// compiler's test, src/shaderc/online/triangle_sample_tests.rs.)
#[test]
fn the_shader_texts_are_glsl_es_100() {
    for src in [VS_SRC, FS_SRC] {
        let src = src.strip_suffix('\0').unwrap();
        assert!(!src.contains('\0'));
        assert_eq!(src.lines().next(), Some("#version 100"), "{src}");
        for desktop in ["layout(", "#version 4", " in vec", " out vec"] {
            assert!(!src.contains(desktop), "{desktop:?} in {src}");
        }
    }
    assert!(VS_SRC.contains("attribute vec4 a_pos;") && VS_SRC.contains("varying vec4 v_color;"));
    assert!(FS_SRC.contains("gl_FragColor = v_color;"));
}

/// Which offline table holds the texts (the table key is the SHA-256 of the
/// text glShaderSource gets, src/shaderc/offline.rs; the header lists each
/// entry as `//   <source> <stage>  sha256=<hex>`). The GC7000 table does.
/// The PowerVR table does NOT (2026-10-03): it holds only the `#version 450`
/// texts under these names. So an AM62P image on the table cannot find
/// these shaders until blobs.rs is regenerated with tools/shaders/
/// gl_test_es100/passthrough.vert and interp_color.frag among its inputs
/// (CLAUDE.md: with a board run of gl_test, gl_slint and gl_femtovg behind
/// it); the sample's AM62P image uses the online compiler instead (its
/// boards conf). This assertion flips when the table gains them.
#[test]
fn which_offline_table_holds_the_shader_texts() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../../tests/reference/");
    let entries = [("vs", VS_SRC.strip_suffix('\0').unwrap()), ("fs", FS_SRC.strip_suffix('\0').unwrap())];
    let has = |table: &str, stage: &str, src: &str| {
        let text = std::fs::read_to_string(format!("{root}{table}")).unwrap();
        let tail = format!(" {stage}  sha256={}", hex(&sha256(src.as_bytes())));
        text.lines().any(|l| l.starts_with("//   ") && l.ends_with(&tail))
    };
    for (stage, src) in entries {
        assert!(has("vivante_blobs.rs", stage, src), "the GC7000 table lost the {stage} text");
        assert!(
            !has("blobs.rs", stage, src),
            "the PowerVR table now holds the {stage} text: the AM62P image can find it; update this test"
        );
    }
    // the search finds a known member: the #version 450 passthrough.vert in both
    let known = "6761eb440b7965b2bd14963f48a252f38d0305d707fbd1d183e187cb76c889b8";
    for table in ["blobs.rs", "vivante_blobs.rs"] {
        let text = std::fs::read_to_string(format!("{root}{table}")).unwrap();
        assert!(text.lines().any(|l| l.ends_with(&format!(" vs  sha256={known}"))), "{table}");
    }
    // the digests of the ES 1.00 texts (the GC7000 table's header, 2026-10-03)
    assert!(hex(&sha256(entries[0].1.as_bytes())).starts_with("9d8d712b"));
    assert!(hex(&sha256(entries[1].1.as_bytes())).starts_with("ec04a3f4"));
}
