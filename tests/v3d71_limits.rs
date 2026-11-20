// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/limits71.rs` (the V3D 7.1 limits and tile-size choice) against
//! Mesa 26.2.1's own sources.
//!
//! The vendored `v3d_limits.h` and `v3d_util.c` are read live (no cc
//! harness: `v3d_util.c` needs Mesa's generated headers, which are not
//! vendored). The checks:
//!
//! * the limit consts equal the `ver >= 71` arms of `V3D_MAX_RENDER_TARGETS`
//!   and `V3D_MAX_FRAMEBUFFER_SIZE`, and `V3D_MAX_IMAGE_DIMENSION`;
//! * the GL answers equal what the GL core reports (`src/gl/state.rs`);
//! * the TLB sizes and the tile table equal `v3d_util.c`'s;
//! * the tile choice equals a line-by-line transliteration of Mesa's 7.1
//!   branch for every total bpp, MSAA and double-buffer setting, and a set
//!   of hand-worked cases.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_limits
//! ```
//! (the module and this test use no crate feature; the set only has to
//! build the library, which `host-test` alone does not.)
#[path = "../src/v3d/limits71.rs"]
mod limits71;

use limits71::*;

const MESA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/mesa-26.2.1/src/broadcom/common");

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The body of `#define NAME ...` in `src`, with its line number.
fn define<'a>(src: &'a str, name: &str) -> (usize, &'a str) {
    let pat = format!("#define {name}");
    src.lines()
        .enumerate()
        .find_map(|(i, l)| {
            let rest = l.strip_prefix(&pat)?;
            (rest.starts_with(' ') || rest.starts_with('(')).then(|| (i + 1, rest.trim()))
        })
        .unwrap_or_else(|| panic!("no #define {name}"))
}

/// A C integer expression of the form `N` or `N * M` (with an optional `u`).
fn c_int(expr: &str) -> u32 {
    let expr = expr.trim().trim_start_matches('(').trim_end_matches(')');
    expr.split('*')
        .map(|t| t.trim().trim_end_matches('u').parse::<u32>().unwrap_or_else(|e| panic!("{t}: {e}")))
        .product()
}

#[test]
fn limits_are_mesa_ver71_arms() {
    let h = read(&format!("{MESA}/v3d_limits.h"));
    let (line, rt) = define(&h, "V3D_MAX_RENDER_TARGETS");
    assert_eq!(line, 44);
    assert_eq!(rt, "(ver) (ver < 71 ? 4 : 8)");
    assert_eq!(MAX_RENDER_TARGETS71, 8);

    let (line, fb) = define(&h, "V3D_MAX_FRAMEBUFFER_SIZE");
    assert_eq!(line, 63);
    assert_eq!(fb, "(ver) (ver < 71 ? 4096u : 7680u)");
    assert_eq!(MAX_FRAMEBUFFER_SIZE71, 7680);

    let (line, img) = define(&h, "V3D_MAX_IMAGE_DIMENSION");
    assert_eq!(line, 62);
    assert_eq!(c_int(img), MAX_IMAGE_DIMENSION71);
}

#[test]
fn gl_answers_match_the_gl_core() {
    let s = read(concat!(env!("CARGO_MANIFEST_DIR"), "/src/gl/state.rs"));
    assert!(s.contains("const MAX_VIEWPORT_DIM: GLsizei = 4096;"), "src/gl/state.rs MAX_VIEWPORT_DIM moved");
    assert!(s.contains("GL_MAX_RENDERBUFFER_SIZE => 4096,"), "src/gl/state.rs GL_MAX_RENDERBUFFER_SIZE arm moved");
    assert_eq!(GL_MAX_TEXTURE_SIZE71, 4096);
    assert_eq!(GL_MAX_RENDERBUFFER_SIZE71, 4096);
    assert_eq!(GL_MAX_VIEWPORT_DIMS71, [4096, 4096]);
    /* every GL-visible size is a target the 7.1 framebuffer holds */
    const {
        assert!(GL_MAX_RENDERBUFFER_SIZE71 <= MAX_FRAMEBUFFER_SIZE71);
        assert!(GL_MAX_VIEWPORT_DIMS71[0] <= MAX_FRAMEBUFFER_SIZE71);
        assert!(GL_MAX_VIEWPORT_DIMS71[1] <= MAX_FRAMEBUFFER_SIZE71);
        assert!(GL_MAX_TEXTURE_SIZE71 <= MAX_IMAGE_DIMENSION71);
    }
}

#[test]
fn tlb_sizes_and_tile_table_are_mesas() {
    let c = read(&format!("{MESA}/v3d_util.c"));
    assert_eq!(c_int(define(&c, "V3D71_TLB_COLOR_SIZE").1), TLB_COLOR_SIZE71);
    assert_eq!(c_int(define(&c, "V3D71_TLB_DETPH_SIZE").1), TLB_DEPTH_SIZE71);
    assert_eq!(c_int(define(&c, "V3D71_TLB_AUX_DETPH_SIZE").1), TLB_AUX_DEPTH_SIZE71);

    let start = c.find("static const uint8_t tile_sizes[] = {").expect("tile_sizes");
    let body = &c[start..];
    let body = &body[body.find('{').unwrap() + 1..body.find("};").unwrap()];
    let nums: Vec<u32> = body
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().unwrap())
        .collect();
    let table: Vec<(u32, u32)> = nums.chunks(2).map(|p| (p[0], p[1])).collect();
    assert_eq!(table, TILE_SIZES71);

    /* the 7.1 branch is the one transliterated in `mesa71` below */
    for needle in [
        "const uint32_t color_bpp = total_color_bpp * (msaa ? 4 : 1);",
        "const uint32_t depth_bpp = 4 * (msaa ? 4 : 1);",
        "if (tile_size_valid(tile_w * tile_h, color_bpp, depth_bpp))",
        "} while (idx < ARRAY_SIZE(tile_sizes) / 2);",
        "assert(idx < ARRAY_SIZE(tile_sizes) / 2);",
    ] {
        assert!(c.contains(needle), "v3d_util.c no longer has `{needle}`");
    }
}

/// Mesa's 7.1 branch of `v3d_choose_tile_size` line by line, its assert as
/// `None`.
fn mesa71(total_color_bpp: u32, msaa: bool, double_buffer: bool) -> Option<(u32, u32)> {
    let valid = |pixel_count: u32, color_bpp: u32, depth_bpp: u32| {
        if pixel_count * depth_bpp <= 8 * 1024 && pixel_count * color_bpp <= 16 * 1024 + 16 * 1024 {
            return true;
        }
        pixel_count * depth_bpp <= 16 * 1024 && pixel_count * color_bpp <= 16 * 1024
    };
    let sizes = [64, 64, 64, 32, 32, 32, 32, 16, 16, 16, 16, 8, 8, 8];
    let color_bpp = total_color_bpp * if msaa { 4 } else { 1 };
    let depth_bpp = 4 * if msaa { 4 } else { 1 };
    let mut idx = 0;
    loop {
        if valid(sizes[idx * 2] * sizes[idx * 2 + 1], color_bpp, depth_bpp) {
            break;
        }
        idx += 1;
        if idx >= sizes.len() / 2 {
            break;
        }
    }
    if double_buffer {
        idx += 1;
    }
    (idx < sizes.len() / 2).then(|| (sizes[idx * 2], sizes[idx * 2 + 1]))
}

#[test]
fn tile_choice_equals_mesa_for_every_input() {
    for total in 0..=1024 {
        for msaa in [false, true] {
            for db in [false, true] {
                assert_eq!(choose_tile_size71_total(total, msaa, db), mesa71(total, msaa, db), "{total} {msaa} {db}");
            }
        }
    }
    for rts in 1..=MAX_RENDER_TARGETS71 {
        for bpp in [Bpp71::Bpp32, Bpp71::Bpp64, Bpp71::Bpp128] {
            for msaa in [false, true] {
                for db in [false, true] {
                    let want = mesa71(rts * bpp.bytes(), msaa, db);
                    assert_eq!(choose_tile_size71(rts, bpp, msaa, db), want, "{rts} {bpp:?} {msaa} {db}");
                    /* a single-buffered frame of at most 8 targets always gets a tile */
                    if !db {
                        assert!(want.is_some(), "{rts} {bpp:?} {msaa}");
                    }
                }
            }
        }
    }
}

#[test]
fn tile_choice_worked_cases() {
    use Bpp71::*;
    /* one 32-bpp target: 64x64 x 4 B is the colour buffer exactly, depth
     * the main buffer exactly */
    assert_eq!(choose_tile_size71(1, Bpp32, false, false), Some((64, 64)));
    assert_eq!(choose_tile_size71(1, Bpp32, false, true), Some((64, 32)));
    /* two 32-bpp targets: 64x32 x 8 B, depth 8 KB in the aux buffer */
    assert_eq!(choose_tile_size71(2, Bpp32, false, false), Some((64, 32)));
    /* one 32-bpp target with MSAA: 16 B colour, 16 B depth per pixel */
    assert_eq!(choose_tile_size71(1, Bpp32, true, false), Some((32, 32)));
    /* four 32-bpp targets with MSAA: 32x16, aux depth 8 KB, colour 32 KB */
    assert_eq!(choose_tile_size71(4, Bpp32, true, false), Some((32, 16)));
    /* eight 128-bpp targets with MSAA, Mesa's most expensive case: 8x8 */
    assert_eq!(choose_tile_size71(8, Bpp128, true, false), Some((8, 8)));
    assert_eq!(choose_tile_size71(8, Bpp128, true, true), None);
    /* out of range */
    assert_eq!(choose_tile_size71(0, Bpp32, false, false), None);
    assert_eq!(choose_tile_size71(9, Bpp32, false, false), None);
    assert_eq!(choose_tile_size71_total(513, true, false), None);
}
