//! Host check of `src/v3d/limits71.rs` against Mesa's V3D 7.1 rules.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_limits`
//! (the features only let the crate build; the test needs none of it).
//!
//! The module is included with `#[path]` (it is not in `src/v3d/mod.rs`
//! yet). Figures are checked two ways: the constants and the tile table are
//! read out of the vendored Mesa sources by text, and the tile choice is
//! compared over the grid (1..=8 RTs) x (4/8/16 bytes = 32/64/128 bpp) x MSAA
//! x double buffer against an independent oracle plus hand-derived anchors
//! from `third_party/mesa-26.2.1/src/broadcom/common/v3d_util.c:106-206`.
//! (Compiling v3d_util.c itself needs Mesa's util headers; not practical
//! with plain cc here.)
#![allow(dead_code)]

#[path = "../src/v3d/limits71.rs"]
mod limits71;
use limits71::*;

fn mesa(path: &str) -> String {
    let p = format!("{}/third_party/mesa-26.2.1/src/broadcom/{}", env!("CARGO_MANIFEST_DIR"), path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn has_line(src: &str, n: usize, needle: &str) {
    let l = src.lines().nth(n - 1).unwrap_or("");
    assert!(l.contains(needle), "line {n} is {l:?}, expected {needle:?}");
}

#[test]
fn constants_match_mesa_text() {
    let h = mesa("common/v3d_limits.h");
    has_line(&h, 41, "V3D_MAX_SAMPLES 4");
    has_line(&h, 43, "V3D_MAX_DRAW_BUFFERS 8");
    has_line(&h, 44, "V3D_MAX_RENDER_TARGETS(ver) (ver < 71 ? 4 : 8)");
    has_line(&h, 62, "V3D_MAX_IMAGE_DIMENSION 4096");
    has_line(&h, 63, "V3D_MAX_FRAMEBUFFER_SIZE(ver) (ver < 71 ? 4096u : 7680u)");
    has_line(&h, 69, "V3D_MAX_MIP_LEVELS 13");
    let u = mesa("common/v3d_util.c");
    has_line(&u, 101, "V3D71_TLB_COLOR_SIZE     (16 * 1024)");
    has_line(&u, 102, "V3D71_TLB_DETPH_SIZE     (16 * 1024)");
    has_line(&u, 103, "V3D71_TLB_AUX_DETPH_SIZE  (8 * 1024)");
    has_line(&u, 169, "total_color_bpp * (msaa ? 4 : 1)");
    has_line(&u, 170, "4 * (msaa ? 4 : 1)");
    has_line(&u, 180, "idx += 1");
    // tile_sizes[] pairs, source lines 134-140.
    for (i, (w, h)) in TILE_SIZES.iter().enumerate() {
        let row: String = u.lines().nth(133 + i).unwrap().chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(row.trim_end_matches(','), format!("{},{}", w, h), "tile_sizes row {i}");
    }
}

#[test]
fn figures() {
    assert_eq!(MAX_RENDER_TARGETS, 8);
    assert_eq!(MAX_DRAW_BUFFERS, 8);
    assert_eq!(MAX_TARGET, 7680);
    assert_eq!(GL_MAX_TEXTURE_SIZE, 4096);
    assert_eq!(GL_MAX_RENDERBUFFER_SIZE, 4096);
    assert_eq!(GL_MAX_VIEWPORT_DIMS, 4096);
    assert_eq!(1u32 << (MAX_MIP_LEVELS - 1), GL_MAX_TEXTURE_SIZE);
    assert_eq!(MAX_SAMPLES, 4);
}

/// Oracle written from the Mesa text, not from the module.
fn oracle(bpp: u32, msaa: bool, db: bool) -> Option<(u32, u32)> {
    let sizes = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
    let m = if msaa { 4 } else { 1 };
    let (c, d) = (bpp * m, 4 * m);
    let mut idx = 0;
    loop {
        let px = sizes[idx].0 * sizes[idx].1;
        let aux = px * d <= 8192 && px * c <= 32768;
        let main = px * d <= 16384 && px * c <= 16384;
        if aux || main {
            break;
        }
        idx += 1;
        if idx >= 7 {
            break;
        }
    }
    if db {
        idx += 1;
    }
    sizes.get(idx).copied()
}

#[test]
fn tile_grid() {
    for rts in 1..=8u32 {
        for bytes in [4u32, 8, 16] {
            for msaa in [false, true] {
                for db in [false, true] {
                    let total = rts * bytes;
                    assert_eq!(
                        choose_tile_size(total, msaa, db),
                        oracle(total, msaa, db),
                        "rts {rts} bytes {bytes} msaa {msaa} db {db}"
                    );
                }
            }
        }
    }
}

#[test]
fn tile_anchors() {
    // Hand-derived from v3d_util.c:106-119 (see the module's docs).
    assert_eq!(choose_tile_size(4, false, false), Some((64, 64)));
    assert_eq!(choose_tile_size(8, false, false), Some((64, 32)));
    assert_eq!(choose_tile_size(16, false, false), Some((64, 32)));
    assert_eq!(choose_tile_size(32, false, false), Some((32, 32)));
    assert_eq!(choose_tile_size(64, false, false), Some((32, 16)));
    assert_eq!(choose_tile_size(128, false, false), Some((16, 16)));
    assert_eq!(choose_tile_size(4, false, true), Some((64, 32)));
    assert_eq!(choose_tile_size(4, true, false), Some((32, 32)));
    // 8 x 128 bpp + MSAA, "the most expensive configuration" (:155-156).
    // 128 bytes x 4 = 512 B/px: only 8x8 fits (64 * 512 = 32 KiB aux-extended).
    assert_eq!(choose_tile_size(128, true, false), Some((8, 8)));
    assert_eq!(choose_tile_size(128, true, true), None);
    // Nothing fits, or double buffer steps past the table: refused.
    assert_eq!(choose_tile_size(1 << 16, false, false), None);
}

#[test]
fn job_tile_bound() {
    let t = choose_tile_size(16, false, false).unwrap();
    let (x, y) = tiles(MAX_TARGET, MAX_TARGET, t);
    assert!(x * y <= MAX_JOB_TILES);
    assert_eq!(MAX_JOB_TILES, 240 * 240);
    assert_eq!(tiles(MAX_TARGET, MAX_TARGET, (32, 32)), (240, 240));
    assert_eq!(tiles(33, 1, (32, 32)), (2, 1));
    // 4.2's bound for comparison: (4096/32)^2.
    const { assert!(MAX_JOB_TILES > (4096 / 32) * (4096 / 32)) };
}
