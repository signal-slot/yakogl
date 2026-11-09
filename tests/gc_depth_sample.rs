// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the descriptor of the depth-sampling experiment (gl_test case
//! `dtpu`, src/vivante/depth_sample.rs; docs/notes/gc-depth-sample.md, the
//! "Descriptor fields" table and the case's knob modes 1 and 2), against the
//! register meaning of the rnndb header (driver/src/vivante/hw/common_3d.xml.h,
//! written out below so a wrong constant in regs.rs shows).
//!
//! The binding `depth_sample::view_binding` makes for a depth surface is what
//! `texture::emit_slot` hands to `build_descriptor`, so the descriptor's words
//! follow from it: CONFIG0 and CONFIG1 are `texture::format_words` of it
//! (LINEAR: it has no CPU texels, so no tiled copy), `LINEAR_STRIDE` is
//! `stride_texels * gles_texfmt_bytes(format)`, `SIZE` is its width and height,
//! LOD 0 its `base_gpu`, and one level. src/vivante/backend_tests.rs reads the
//! same words out of a submitted stream for a surface a draw allocated.
//!
//! What it cannot show: whether the texture unit reads the PE's row-linear
//! D24S8 words as D24X8. That is the board case.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_sample
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use yakogl::backend::{gles_texfmt_bytes, GlesTextureBind, GLES_TEXFMT_R32F, GLES_WRAP_CLAMP_TO_EDGE};
use yakogl::util::ENOTSUP;
use yakogl::vivante::depth_sample::*;
use yakogl::vivante::texture::{format_words, gc_texfmt};

/// common_3d.xml.h:51 and :61, TEXTURE_FORMAT.
const FORMAT_A8R8G8B8: u32 = 0x07;
const FORMAT_D24X8: u32 = 0x11;
/// common_3d.xml.h:132, TEXTURE_ADDRESSING_MODE_LINEAR.
const ADDRESSING_LINEAR: u32 = 3;
/// common_3d.xml.h TEXTURE_TYPE 2D.
const TYPE_2D: u32 = 2;
/// common_3d.xml.h:118-123, TEXTURE_SWIZZLE.
const SW_R: u32 = 0;
const SW_G: u32 = 1;
const SW_B: u32 = 2;
const SW_A: u32 = 3;
const SW_1: u32 = 5;

/// The words of the descriptor `build_descriptor` writes for `b`, by field.
#[derive(Debug, PartialEq, Eq)]
struct Desc {
    type_: u32,
    format: u32,
    addressing: u32,
    format_ext: u32,
    swizzle: (u32, u32, u32, u32),
    linear_stride: u32,
    size: (u32, u32),
    lod0: u64,
    levels: u8,
    int_filter: bool,
}

fn desc(b: &GlesTextureBind) -> Desc {
    let f = gc_texfmt(b.format);
    assert!(!f.compressed && b.cpu.is_null(), "LINEAR: no block format and no tiled copy");
    let [c0, c1, _] = format_words(b, false);
    Desc {
        type_: c0 & 7,
        format: (c0 >> 13) & 0x1f,
        addressing: (c0 >> 20) & 3,
        format_ext: c1 & 0x3f,
        swizzle: ((c1 >> 8) & 7, (c1 >> 12) & 7, (c1 >> 16) & 7, (c1 >> 20) & 7),
        linear_stride: b.stride_texels * gles_texfmt_bytes(b.format),
        size: (b.width, b.height),
        lod0: b.base_gpu,
        levels: b.mip_levels,
        /* int_filter_wanted: not a 3D texture, and the class allows it */
        int_filter: f.int_filter,
    }
}

/// A `w` x `h` depth texture's binding as the GL core builds it after the
/// answer: its R32F texels (stale), NEAREST, CLAMP_TO_EDGE, the depth id.
fn depth_texture(w: u32, h: u32, depth_luminance: u8) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_R32F,
        base_gpu: 0x4800_0000,
        width: w,
        height: h,
        stride_texels: w,
        bytes: w * h * 4,
        mip_levels: 1,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        no_alpha: 1,
        depth_luminance,
        depth_id: 0x4000_0064,
        cpu: 0x1000 as *const core::ffi::c_void,
        ..GlesTextureBind::ZERO
    }
}

/// A surface as ds_cache keeps one: 64-aligned both ways, past a guard.
fn surface(w: u32, h: u32) -> DepthView {
    DepthView { gpu: 0x5000_0000 + 256 * 1024, stride: w.next_multiple_of(64) * 4, rows: h.next_multiple_of(64) }
}

/// Mode 1, route 2 of the note: D24X8 (0x11) in CONFIG0 and FORMAT_EXT 0,
/// LINEAR, (R, R, R, 1), no INT_FILTER, LINEAR_STRIDE the surface's stride,
/// SIZE the texture's, LOD 0 the surface past the guard, one level. The same
/// whatever the GL surface (ES 2.0's luminance flag or ES 3.0's RED).
#[test]
fn mode_1_describes_the_surface_as_d24x8() {
    for lum in [0, 1] {
        let v = surface(64, 64);
        let b = view_binding(&depth_texture(64, 64, lum), DEPTH_TPU_D24X8, Some(v)).expect("it fits");
        assert_eq!(
            desc(&b),
            Desc {
                type_: TYPE_2D,
                format: FORMAT_D24X8,
                addressing: ADDRESSING_LINEAR,
                format_ext: 0,
                swizzle: (SW_R, SW_R, SW_R, SW_1),
                linear_stride: 256,
                size: (64, 64),
                lod0: v.gpu,
                levels: 1,
                int_filter: false,
            },
            "depth_luminance {lum}"
        );
        assert_eq!((b.depth_id, b.mip_filter), (0, 0));
        assert_eq!((b.wrap_s, b.wrap_t), (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_CLAMP_TO_EDGE), "the texture's wraps");
    }
}

/// Mode 2, the raw view: A8R8G8B8 with the identity swizzle, so the texel is
/// (D1, D0, S, D2) of the word D2 D1 D0 S; INT_FILTER on (a unorm8 format),
/// NEAREST whatever the texture asked; the same address and stride.
#[test]
fn mode_2_describes_the_surface_as_raw_a8r8g8b8() {
    let v = surface(64, 64);
    let mut t = depth_texture(64, 64, 1);
    (t.linear_filter, t.min_linear) = (1, 1);
    let b = view_binding(&t, DEPTH_TPU_RAW8888, Some(v)).expect("it fits");
    assert_eq!(
        desc(&b),
        Desc {
            type_: TYPE_2D,
            format: FORMAT_A8R8G8B8,
            addressing: ADDRESSING_LINEAR,
            format_ext: 0,
            swizzle: (SW_R, SW_G, SW_B, SW_A),
            linear_stride: 256,
            size: (64, 64),
            lod0: v.gpu,
            levels: 1,
            int_filter: true,
        }
    );
    assert_eq!((b.linear_filter, b.min_linear), (0, 0), "NEAREST");
}

/// The surface is wider than the texture (64-aligned): LINEAR_STRIDE is the
/// surface's, SIZE the texture's (90x70 on a 128x128 surface, and 1x1).
#[test]
fn a_padded_surface_keeps_its_stride_and_the_textures_size() {
    for (w, h, stride) in [(90, 70, 512), (1, 1, 256)] {
        let v = surface(w, h);
        let b = view_binding(&depth_texture(w, h, 1), DEPTH_TPU_D24X8, Some(v)).expect("it fits");
        let d = desc(&b);
        assert_eq!((d.linear_stride, d.size, d.lod0), (stride, (w, h), v.gpu), "{w}x{h}");
    }
}

/// No surface, a texture wider or taller than it, mode 0 or an unknown mode:
/// -ENOTSUP, never the stale texels.
#[test]
fn a_texture_that_does_not_fit_the_live_surface_is_refused() {
    let v = surface(64, 64);
    let t = depth_texture(64, 64, 1);
    assert_eq!(view_binding(&t, DEPTH_TPU_D24X8, None).err(), Some(-ENOTSUP), "no live surface");
    assert_eq!(view_binding(&depth_texture(65, 64, 1), DEPTH_TPU_D24X8, Some(v)).err(), Some(-ENOTSUP), "wider");
    assert_eq!(view_binding(&depth_texture(64, 65, 1), DEPTH_TPU_RAW8888, Some(v)).err(), Some(-ENOTSUP), "taller");
    assert_eq!(view_binding(&depth_texture(0, 64, 1), DEPTH_TPU_D24X8, Some(v)).err(), Some(-ENOTSUP), "empty");
    assert_eq!(view_binding(&t, DEPTH_TPU_OFF, Some(v)).err(), Some(-ENOTSUP), "mode 0");
    assert_eq!(view_binding(&t, 3, Some(v)).err(), Some(-ENOTSUP), "no mode 3");
    assert!(v.covers(64, 64) && !v.covers(65, 64) && !v.covers(64, 65));
}

/// Without the knob (every product build, and a test-knobs build that has
/// not set it) the GC answers no: the GL core keeps the CPU resolve.
#[test]
fn the_answer_is_no_while_the_knob_is_off() {
    assert_eq!(mode(), DEPTH_TPU_OFF);
    assert!(!gc_depth_sampled(0x4000_0064, 64, 64));
    assert!(!yakogl::backend::samples_rendered_depth(0x4000_0064, 64, 64));
}
