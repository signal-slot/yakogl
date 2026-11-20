// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 uniform stream fill (src/v3d/uniform71.rs): the streams of
//! ours_color_uv and ours_clear for the viewport and clear values of gl_test
//! cases 5, 58, 59, 66, 67 and 102, against the uniform words in Mesa's 7.1
//! references (tools/v3d/pi5/gl_*.clif), and every refusal with synthetic
//! input.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_uniform`

#![allow(dead_code)]

#[path = "../src/v3d/uniform71.rs"]
mod uniform71;

use uniform71::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

/// The `"uniforms": [...]` list of stage `fs`, `vs` or `cs` in a program json.
fn list(json: &str, stage: &str) -> Vec<Word> {
    let at = json.find(&format!("\"{stage}\": {{")).unwrap();
    let rest = &json[at..];
    let u = rest.find("\"uniforms\": [").unwrap() + "\"uniforms\": [".len();
    let body = &rest[u..u + rest[u..].find(']').unwrap()];
    body.split('{')
        .skip(1)
        .map(|e| {
            let c = e.split("\"contents\":").nth(1).unwrap().split(',').next().unwrap().trim();
            let d = e.split("\"data\": \"").nth(1).unwrap().split('"').next().unwrap();
            Word { contents: c.parse().unwrap(), data: hex(d) }
        })
        .collect()
}

/// Every `@format binary` block of a Mesa clif as words (uniform streams,
/// shader code, vertex data and so on; the checks look for exact runs).
fn blocks(clif: &str) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = Vec::new();
    let mut on = false;
    for l in clif.lines() {
        let l = l.strip_prefix("MESA: info: ").unwrap_or(l);
        if l.starts_with("@format binary") {
            out.push(Vec::new());
            on = true;
        } else if l.starts_with('@') {
            on = false;
        } else if on && l.starts_with("0x") {
            out.last_mut().unwrap().extend(l.split_whitespace().map(hex));
        }
    }
    out
}

fn has_run(b: &[Vec<u32>], run: &[u32]) -> bool {
    b.iter().any(|x| x.windows(run.len()).any(|w| w == run))
}

fn starts_block(b: &[Vec<u32>], run: &[u32]) -> bool {
    b.iter().any(|x| x.starts_with(run))
}

fn f(x: f32) -> u32 {
    x.to_bits()
}

const CASES: [(&str, &str); 6] = [
    ("5 case_scissor", "gl_scissor"),
    ("58 case_clearscis", "gl_scisbox"),
    ("59 case_clearmask", "gl_writemask"),
    ("66 case_clearvp", "gl_viewport"),
    ("67 case_fbods", "gl_fbods"),
    ("102 case_maskclear", "gl_colormask"),
];

/// Every case draws into a 64x64 target at the default viewport and depth
/// range: scale (32, 32, 0.5), Z translate 0.5.
fn vp64() -> Viewport {
    Viewport::new(64, 64, 0.0, 1.0)
}

#[test]
fn json_lists_have_the_expected_shape() {
    let clear = read("tools/v3d/gltest-shaders71/ours_clear.json");
    assert_eq!(list(&clear, "fs"), (0..4).map(|i| Word { contents: UNIFORM, data: i }).collect::<Vec<_>>());
    assert_eq!(list(&clear, "vs").len(), 12);
    assert_eq!(list(&clear, "cs").len(), 12);
    let cuv = read("tools/v3d/gltest-shaders71/ours_color_uv.json");
    assert!(list(&cuv, "fs").is_empty());
    assert_eq!(list(&cuv, "vs").len(), 24);
    assert_eq!(list(&cuv, "cs").len(), 12);
}

#[test]
fn clear_streams_are_filled() {
    let json = read("tools/v3d/gltest-shaders71/ours_clear.json");
    let vp = vp64();
    // glClearColor(0, 0, 1, 1)
    let col = [f(0.0), f(0.0), f(1.0), f(1.0)];
    let mut out = [0u32; 32];
    let n = fill(&list(&json, "fs"), &vp, &col, &mut out).unwrap();
    assert_eq!(&out[..n], &col);
    // VS: vattr loads 0..3 as constants, X, 0, Y, 1, Z scale, Z offset, 2, 3
    let n = fill(&list(&json, "vs"), &vp, &[], &mut out).unwrap();
    assert_eq!(&out[..n], &[0, 1, 2, 3, f(2048.0), 0, f(2048.0), 1, f(0.5), f(0.5), 2, 3],);
    // CS: 0..3, 0..3, X, 4, Y, 5 (no UNIFORM, no Z words)
    let n = fill(&list(&json, "cs"), &vp, &[], &mut out).unwrap();
    assert_eq!(&out[..n], &[0, 1, 2, 3, 0, 1, 2, 3, f(2048.0), 4, f(2048.0), 5]);
}

#[test]
fn color_uv_streams_are_filled() {
    let json = read("tools/v3d/gltest-shaders71/ours_color_uv.json");
    let vp = vp64();
    let mut out = [0u32; 32];
    assert_eq!(fill(&list(&json, "fs"), &vp, &[], &mut out), Ok(0));
    let n = fill(&list(&json, "vs"), &vp, &[], &mut out).unwrap();
    let mut want: Vec<u32> = (0..10).collect();
    want.extend([f(2048.0), 0, f(2048.0), 1, f(0.5), f(0.5)]);
    want.extend(2..10);
    assert_eq!(&out[..n], &want[..]);
    let n = fill(&list(&json, "cs"), &vp, &[], &mut out).unwrap();
    assert_eq!(&out[..n], &[0, 1, 2, 3, 0, 1, 2, 3, f(2048.0), 4, f(2048.0), 5]);
}

#[test]
fn viewport_words_match_mesa_in_the_six_cases() {
    // Mesa's blitter programs read X scale, Y scale, Z offset and Z scale in
    // their vertex streams: 0x45000000 twice (32 * 64 each) and 0.5 twice,
    // either consecutive or as X, 3 words, Y, 1 word, 0.5, 0.5 (the clear
    // quad's order).
    let run = [f(2048.0), f(2048.0), f(0.5), f(0.5)];
    let (x, h) = (f(2048.0), f(0.5));
    let vp = vp64();
    let w = |c| word(Word { contents: c, data: 0 }, &vp, &[]).unwrap();
    assert_eq!([w(VIEWPORT_X_SCALE), w(VIEWPORT_Y_SCALE), w(VIEWPORT_Z_OFFSET), w(VIEWPORT_Z_SCALE)], run);
    for (case, name) in CASES {
        let b = blocks(&read(&format!("tools/v3d/pi5/{name}.clif")));
        let spread = b.iter().any(|v| v.windows(8).any(|w| w[0] == x && w[4] == x && w[6] == h && w[7] == h));
        assert!(has_run(&b, &run) || spread, "case {case}: {name}.clif has no viewport words");
    }
}

#[test]
fn clear_colours_match_mesa_fs_streams() {
    // Mesa's clear quad's fragment stream is the colour's four words, at
    // the start of the stream block. (clif file, colour)
    let one = f(1.0);
    let half = f(0.5);
    let want: [(&str, [u32; 4]); 5] = [
        ("gl_scisbox", [one, 0, 0, one]),        // case 58: the red clear
        ("gl_viewport", [0, one, 0, one]),       // case 66: the green clear
        ("gl_colormask", [one, one, one, half]), // case 102: R+A masked clear
        ("gl_colormask", [0, 0, one, 0]),        // case 102: B-only clear
        ("gl_colormask", [0, one, 0, 0]),        // case 102: G-only clear
    ];
    let json = read("tools/v3d/gltest-shaders71/ours_clear.json");
    let vp = vp64();
    for (name, col) in want {
        let b = blocks(&read(&format!("tools/v3d/pi5/{name}.clif")));
        assert!(starts_block(&b, &col), "{name}: no stream starting {col:x?}");
        let mut out = [0u32; 4];
        let n = fill(&list(&json, "fs"), &vp, &col, &mut out).unwrap();
        assert_eq!(&out[..n], &col);
    }
}

#[test]
fn smaller_viewport_and_depth_range() {
    // Not in the references: a 32x16 viewport, depth range 0.25..0.75.
    let vp = Viewport::new(32, 16, 0.25, 0.75);
    let w = |c| word(Word { contents: c, data: 0 }, &vp, &[]).unwrap();
    assert_eq!(w(VIEWPORT_X_SCALE), f(16.0 * 64.0));
    assert_eq!(w(VIEWPORT_Y_SCALE), f(8.0 * 64.0));
    assert_eq!(w(VIEWPORT_Z_SCALE), f(0.25));
    assert_eq!(w(VIEWPORT_Z_OFFSET), f(0.5));
}

#[test]
fn constants_match_the_other_tables() {
    let ffi = read("src/v3d/ffi.rs");
    let sel = read("src/shaderc/online/v3d/select.rs");
    for (n, v) in [
        ("CONSTANT", CONSTANT),
        ("UNIFORM", UNIFORM),
        ("VIEWPORT_X_SCALE", VIEWPORT_X_SCALE),
        ("VIEWPORT_Y_SCALE", VIEWPORT_Y_SCALE),
        ("VIEWPORT_Z_OFFSET", VIEWPORT_Z_OFFSET),
        ("VIEWPORT_Z_SCALE", VIEWPORT_Z_SCALE),
    ] {
        assert!(ffi.contains(&format!("pub const V3D_QUNIFORM_{n}: u32 = {v};")), "{n}");
    }
    for (n, v) in [
        ("TMU_CONFIG_P0", TMU_CONFIG_P0),
        ("TMU_CONFIG_P1", TMU_CONFIG_P1),
        ("TEXTURE_WIDTH", TEXTURE_WIDTH),
        ("TEXTURE_HEIGHT", TEXTURE_HEIGHT),
        ("TEXTURE_DEPTH", TEXTURE_DEPTH),
        ("TEXTURE_ARRAY_SIZE", TEXTURE_ARRAY_SIZE),
    ] {
        assert!(sel.contains(&format!("pub const QUNIFORM_{n}: u32 = {v};")), "{n}");
    }
    for (n, v) in [
        ("FB_WPOS_Y_SCALE", FB_WPOS_Y_SCALE),
        ("FB_WPOS_Y_OFFSET", FB_WPOS_Y_OFFSET),
        ("FB_PNTC_Y_SCALE", FB_PNTC_Y_SCALE),
        ("FB_PNTC_Y_OFFSET", FB_PNTC_Y_OFFSET),
    ] {
        assert!(ffi.contains(&format!("pub const V3D_QUNIFORM_{n}: u32 = {v:#x};")), "{n}");
    }
    let c = read("driver/src/v3d/v3d_frame71_tri.c");
    assert!(c.contains("#define CLIPPER_XY_GRANULARITY 64.0f"));
}

#[test]
fn every_refusal_is_named() {
    let vp = vp64();
    let w = |c, d| word(Word { contents: c, data: d }, &vp, &[1, 2]);
    // a UNIFORM word past the block, and with no block
    let e = w(UNIFORM, 2).unwrap_err();
    assert_eq!(e, Refusal::UniformIndex { index: 2, count: 2 });
    assert_eq!(e.to_string(), "uniform word 2 is outside the stage's 2-word GL uniform block");
    assert_eq!(word(Word { contents: UNIFORM, data: 0 }, &vp, &[]), Err(Refusal::UniformIndex { index: 0, count: 0 }));
    // texture state
    for c in [TMU_CONFIG_P0, TMU_CONFIG_P1] {
        let e = w(c, 1 << 24).unwrap_err();
        assert_eq!(e, Refusal::TmuConfig { contents: c });
        assert!(e.to_string().starts_with("TMU config uniform"));
    }
    for c in [TEXTURE_WIDTH, TEXTURE_HEIGHT, TEXTURE_DEPTH, TEXTURE_ARRAY_SIZE] {
        let e = w(c, 0).unwrap_err();
        assert_eq!(e, Refusal::TextureSize { contents: c });
        assert!(e.to_string().starts_with("texture size uniform"));
    }
    // gl_FragCoord.y and gl_PointCoord.y
    for c in [FB_WPOS_Y_SCALE, FB_WPOS_Y_OFFSET, FB_PNTC_Y_SCALE, FB_PNTC_Y_OFFSET] {
        let e = w(c, 0).unwrap_err();
        assert_eq!(e, Refusal::FragCoordY { contents: c });
        assert!(e.to_string().starts_with("gl_FragCoord/gl_PointCoord y transform"));
    }
    // anything else, 6 (the next Mesa kind), 48 (levels) and the far end
    for c in [6, 7, 42, 43, 48, 0x20000, u32::MAX] {
        let e = w(c, 0).unwrap_err();
        assert_eq!(e, Refusal::Unknown { contents: c });
        assert_eq!(e.to_string(), format!("uniform contents {c:#x} is not supported on 7.1"));
    }
    // a short output
    let list = [Word { contents: CONSTANT, data: 7 }; 3];
    let mut out = [0u32; 2];
    let e = fill(&list, &vp, &[], &mut out).unwrap_err();
    assert_eq!(e, Refusal::OutputTooSmall { need: 3, have: 2 });
    assert_eq!(e.to_string(), "uniform stream needs 3 words, room for 2");
    // a refusal in the middle stops the fill
    let mixed = [Word { contents: CONSTANT, data: 1 }, Word { contents: 99, data: 0 }];
    assert_eq!(fill(&mixed, &vp, &[], &mut out), Err(Refusal::Unknown { contents: 99 }));
}
