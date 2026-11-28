// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Properties of `layout::offset` (src/vivante/layout.rs), the byte offset of a
//! pixel in a GC surface layout, for glmark2's sizes (800x600, 1024x1024,
//! 512x512, 256x256), odd mip sizes (1x1, 3x5, 13x7) and sizes that are not a
//! multiple of the tile or the super-tile.
//!
//! For each layout (LINEAR, TILED, SUPER_TILED; both super-tile orders through
//! `offset_in`, and `offset` itself), each bytes per pixel (1, 2, 4, 8, 16)
//! and both `rs_align` settings:
//!
//! 1. every pixel of the padded surface (`aligned_width` x `aligned_height`)
//!    gets its own offset, a multiple of `bpp`, with `offset + bpp <=
//!    layout::size`; and, as the padded surface is full, the offsets cover
//!    `0..size` exactly (a bijection onto the padded surface);
//! 2. LINEAR's offset is `y * stride + x * bpp`.
//!
//! Large sizes are checked on every pixel; a seeded generator (xorshift64, no
//! dependency) adds random samples, also outside the padded area for the
//! linear identity.
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-vivante
//! --test gc_layout_props`.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use yakogl::vivante::layout::{self, Layout, SuperOrder};

const LAYOUTS: [Layout; 3] = [Layout::Linear, Layout::Tiled, Layout::SuperTiled];
const ORDERS: [SuperOrder; 2] = [SuperOrder::Drm, SuperOrder::Interleaved];
const BPPS: [u32; 5] = [1, 2, 4, 8, 16];
const SIZES: [(u32, u32); 14] = [
    (800, 600),
    (1024, 1024),
    (512, 512),
    (256, 256),
    (1, 1),
    (3, 5),
    (13, 7),
    (5, 3),
    (4, 4),
    (63, 65),
    (65, 63),
    (100, 33),
    (17, 1),
    (1, 130),
];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
}

/// Every pixel of the padded surface maps to a distinct bpp-aligned slot, and
/// the slots are exactly `0..size / bpp`.
fn check_bijection(layout: Layout, order: SuperOrder, w: u32, h: u32, bpp: u32, rs: bool) {
    let what = format!("{layout:?} {order:?} {w}x{h} bpp {bpp} rs_align {rs}");
    let (pw, ph) = (layout::aligned_width(layout, w, rs), layout::aligned_height(layout, h, rs));
    let stride = layout::stride(layout, w, bpp, rs);
    let size = layout::size(layout, w, h, bpp, rs);
    assert_eq!(stride, pw * bpp, "{what}: stride");
    assert_eq!(size, (pw * ph * bpp) as usize, "{what}: size");
    let slots = size / bpp as usize;
    let mut seen = vec![false; slots];
    for y in 0..ph {
        for x in 0..pw {
            let o = layout::offset_in(layout, order, x, y, bpp, stride);
            assert_eq!(o % bpp as usize, 0, "{what}: ({x},{y}) offset {o} unaligned");
            assert!(o + bpp as usize <= size, "{what}: ({x},{y}) offset {o} outside size {size}");
            let s = o / bpp as usize;
            assert!(!seen[s], "{what}: ({x},{y}) offset {o} already taken");
            seen[s] = true;
        }
    }
    assert!(seen.iter().all(|&b| b), "{what}: a slot of the padded surface is unused");
}

#[test]
fn offset_is_a_bijection_on_the_padded_surface() {
    for &layout in &LAYOUTS {
        for &(w, h) in &SIZES {
            for &bpp in &BPPS {
                for rs in [false, true] {
                    for &order in &ORDERS {
                        check_bijection(layout, order, w, h, bpp, rs);
                    }
                }
            }
        }
    }
}

#[test]
fn offset_uses_the_default_super_order() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for &layout in &LAYOUTS {
        for &(w, h) in &SIZES {
            for &bpp in &BPPS {
                let stride = layout::stride(layout, w, bpp, true);
                for _ in 0..64 {
                    let (x, y) = (
                        rng.below(layout::aligned_width(layout, w, true)),
                        rng.below(layout::aligned_height(layout, h, true)),
                    );
                    assert_eq!(
                        layout::offset(layout, x, y, bpp, stride),
                        layout::offset_in(layout, layout::SUPER_ORDER, x, y, bpp, stride),
                        "{layout:?} {w}x{h} bpp {bpp} ({x},{y})"
                    );
                }
            }
        }
    }
}

#[test]
fn random_samples_in_the_padded_surface_stay_inside_and_apart() {
    // Seeded random pixel pairs: distinct pixels, distinct offsets, both
    // aligned and inside the padded size.
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for &layout in &LAYOUTS {
        for &order in &ORDERS {
            for &(w, h) in &SIZES {
                for &bpp in &BPPS {
                    for rs in [false, true] {
                        let (pw, ph) = (layout::aligned_width(layout, w, rs), layout::aligned_height(layout, h, rs));
                        let stride = layout::stride(layout, w, bpp, rs);
                        let size = layout::size(layout, w, h, bpp, rs);
                        for _ in 0..200 {
                            let a = (rng.below(pw), rng.below(ph));
                            let b = (rng.below(pw), rng.below(ph));
                            let (oa, ob) = (
                                layout::offset_in(layout, order, a.0, a.1, bpp, stride),
                                layout::offset_in(layout, order, b.0, b.1, bpp, stride),
                            );
                            assert!(oa % bpp as usize == 0 && oa + bpp as usize <= size);
                            assert!(ob % bpp as usize == 0 && ob + bpp as usize <= size);
                            assert_eq!(a == b, oa == ob, "{layout:?} {order:?} {w}x{h} bpp {bpp}: {a:?} vs {b:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn linear_offset_is_y_stride_plus_x_bpp() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    for &(w, h) in &SIZES {
        for &bpp in &BPPS {
            for rs in [false, true] {
                let stride = layout::stride(Layout::Linear, w, bpp, rs);
                // Corners and seeded random pixels; also past the padded
                // area (the formula is plain arithmetic).
                let mut pts = vec![(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)];
                for _ in 0..200 {
                    pts.push((rng.below(w + 70), rng.below(h + 70)));
                }
                for (x, y) in pts {
                    let want = y as usize * stride as usize + x as usize * bpp as usize;
                    for &order in &ORDERS {
                        assert_eq!(
                            layout::offset_in(Layout::Linear, order, x, y, bpp, stride),
                            want,
                            "{w}x{h} bpp {bpp} ({x},{y})"
                        );
                    }
                    assert_eq!(layout::offset(Layout::Linear, x, y, bpp, stride), want);
                }
            }
        }
    }
}
