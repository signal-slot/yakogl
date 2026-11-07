// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/vivante/depth_format.rs`: the depth format, sizes and clear words,
//! against Mesa's etnaviv values for terrain's 800x600 depth16 renderbuffer.

#[path = "../src/vivante/depth_format.rs"]
mod depth_format;

use depth_format::*;

#[test]
fn terrain_depth16_stride_is_mesas() {
    let s = depth_surface(GlDepthAttachment::DepthComponent16).unwrap();
    assert_eq!(s.format, PeDepthFormat::D16);
    assert_eq!(s.stride(800, 64), 0x680);
    assert_eq!(s.size(800, 600, 64), 0x680 * 640);
    let old = depth_surface(GlDepthAttachment::Depth24Stencil8).unwrap();
    assert_eq!(old.stride(800, 64), 0xd00);
    assert_eq!(old.size(800, 600, 64), 2 * s.size(800, 600, 64));
}

#[test]
fn formats_and_read_widths() {
    assert!(depth_surface(GlDepthAttachment::None).is_none());
    for (a, f, b) in [
        (GlDepthAttachment::DepthComponent16, PeDepthFormat::D16, 2),
        (GlDepthAttachment::DepthComponent24, PeDepthFormat::D24S8, 4),
        (GlDepthAttachment::Depth24Stencil8, PeDepthFormat::D24S8, 4),
        (GlDepthAttachment::StencilIndex8, PeDepthFormat::D24S8, 4),
    ] {
        let s = depth_surface(a).unwrap();
        assert_eq!((s.format, s.bytes_per_pixel, s.resolve_read_bytes), (f, b, b));
    }
}

#[test]
fn padding_follows_the_layout() {
    let s = depth_surface(GlDepthAttachment::DepthComponent16).unwrap();
    assert_eq!(s.stride(1, 4), 8);
    assert_eq!(s.size(5, 5, 4), 16 * 8);
    assert_eq!(s.stride(64, 64), 128);
}

#[test]
fn clear_words() {
    let d16 = depth_surface(GlDepthAttachment::DepthComponent16).unwrap();
    assert_eq!(d16.clear_value(1.0, 0xff), 0xffff_ffff);
    assert_eq!(d16.clear_value(0.0, 0), 0);
    assert_eq!(d16.clear_value(0.5, 0), 0x8000_8000);
    assert_eq!(d16.clear_value(2.0, 0), 0xffff_ffff);
    let d24 = depth_surface(GlDepthAttachment::Depth24Stencil8).unwrap();
    assert_eq!(d24.clear_value(1.0, 0x5a), 0xffff_ff5a);
    assert_eq!(d24.clear_value(0.0, 0x01), 0x01);
    assert_eq!(d24.clear_value(-1.0, 0), 0);
}
