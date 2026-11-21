// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's texture state words with GL_TEXTURE_BASE_LEVEL,
//! GL_TEXTURE_MAX_LEVEL, GL_TEXTURE_MIN_LOD and GL_TEXTURE_MAX_LOD
//! (`yakogl::pvr::texture`, `pvr::backend::texture_descriptor_lod`), pinned
//! against texstate.xml's fields as Mesa 26.2.1 fills them:
//!
//! - TEXSTATE_IMAGE_WORD1.baselevel [60:63] and num_mip_levels [0:3] of a
//!   twiddled image (pvr_arch_tex_state.c: the view's base_mip_level and its
//!   level count from there);
//! - a linear image (TEXSTATE_STRIDE_IMAGE_WORD1, no baselevel) stated from
//!   its base level: texaddr [16:53] moved by pvr_image.c's level offset,
//!   IMAGE_WORD0 width/height and the stride minified, num_mip_levels
//!   [60:63] counted from the base level;
//! - TEXSTATE_SAMPLER_WORD0 minlod [13:22] and maxlod [23:32], u4.6:
//!   pvr_arch_sampler.c's `util_unsigned_fixed(CLAMP(lod, 0, 959 / 64), 6)`,
//!   inside the range the levels from the base allow.
//!
//! GL's defaults pack the words the backend packed before (pinned here as
//! literal words, taken from the builder before the change; the same for
//! 32,256 bind/format/filter combinations at the time of the change).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_tex_lod_words
//! ```
#![cfg(feature = "gpu-powervr")]
use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_TEXTYPE_CUBE,
    GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
};
use yakogl::pvr::backend::{texture_descriptor, texture_descriptor_lod};
use yakogl::pvr::texture::TexLod;

const BASE: u64 = 0x80_0010_0000;

/// The bind the default words were recorded with.
fn bind(format: u8, type_: u8, w: u32, h: u32, mip_filter: u8, mip_levels: u8, filt: (u8, u8)) -> GlesTextureBind {
    GlesTextureBind {
        format,
        type_,
        layers: if type_ == GLES_TEXTYPE_2D { 1 } else { 6 },
        base_gpu: BASE,
        width: w,
        height: h,
        stride_texels: w.next_power_of_two(),
        bytes: w.next_power_of_two() * h * 4 * 6,
        linear_filter: filt.0,
        min_linear: filt.1,
        mip_filter,
        mip_levels,
        wrap_s: 1,
        wrap_t: 2,
        ..GlesTextureBind::ZERO
    }
}

fn lod(base_level: u32, max_level: u32, min_lod: f32, max_lod: f32) -> TexLod {
    TexLod { base_level, max_level, min_lod, max_lod }
}

fn word(w: &[u32], i: usize) -> u64 {
    u64::from(w[i]) | (u64::from(w[i + 1]) << 32)
}

fn bits(v: u64, lo: u32, hi: u32) -> u64 {
    (v >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}

/* ---- Mesa's formulas ---- */

/// pvr_arch_sampler.c: `util_unsigned_fixed(CLAMP(lod, 0.0f,
/// ROGUE_TEXSTATE_CLAMP_MAX / (1 << 6)), 6)`, CLAMP_MAX 959.
fn mesa_lod(lod: f32) -> u64 {
    (lod.clamp(0.0, 959.0 / 64.0) * 64.0) as u64
}

/// pvr_image.c pvr_image_plane_setup_mip_levels: level l of a mipmapped
/// image of power-of-two physical extent pw x ph, cpp bytes a texel.
fn mesa_level_offset(pw: u32, ph: u32, cpp: u32, level: u32) -> u64 {
    (0..level).map(|k| u64::from(cpp * (pw >> k).max(1) * (ph >> k).max(1))).sum()
}

/// The fields a test reads.
struct Fields {
    width: u64,
    height: u64,
    iw1: u64,
    minlod: u64,
    maxlod: u64,
    layer_size: u32,
}

fn fields(w: &[u32]) -> Fields {
    let iw0 = word(w, 0);
    let sw0 = word(w, 8);
    Fields {
        width: bits(iw0, 34, 47) + 1,
        height: bits(iw0, 48, 61) + 1,
        iw1: word(w, 2),
        minlod: bits(sw0, 13, 22),
        maxlod: bits(sw0, 23, 32),
        layer_size: w[4],
    }
}

fn texaddr(iw1: u64) -> u64 {
    bits(iw1, 16, 53) << 2
}

/* ---- GL's defaults: today's words ---- */

#[test]
fn the_defaults_pack_the_words_of_before() {
    assert_eq!(TexLod::default(), lod(0, 1000, -1000.0, 1000.0));
    let cases: [(GlesTextureBind, [u32; 20]); 4] = [
        (
            bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 2, 7, (1, 1)),
            [
                0x60000a64, 0x003f00fc, 0x0000803f, 0x78200004, 0x00004000, 0, 0, 0x00000140, 0xc0000fff, 0x02002350,
                0, 0, 0, 0x00000611, 0, 0, 0, 0, 0, 0,
            ],
        ),
        (
            bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 0, 1, (0, 1)),
            [
                0x60000a64, 0x003f00fc, 0x0000003f, 0x18200004, 0x00004000, 0, 0, 0x00000140, 0x20000fff, 0x02002240,
                0, 0, 0, 0x00000003, 0, 0, 0, 0, 0, 0,
            ],
        ),
        (
            bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, 64, 64, 2, 7, (1, 1)),
            [
                0x60000a63, 0x003f00fc, 0x00008057, 0x08200004, 0x00004000, 0, 0, 0x00000140, 0xc0000fff, 0x02002350,
                0, 0, 0, 0x00000611, 0, 0, 0, 0, 0, 0,
            ],
        ),
        (
            bind(GLES_TEXFMT_ETC2_RGB, GLES_TEXTYPE_2D, 64, 64, 2, 7, (1, 1)),
            [
                0x20000a81, 0x003f00fe, 0x00008007, 0x00600004, 0x00018000, 0, 0, 0x0000011f, 0xc0000fff, 0x02002350,
                0, 0, 0, 0x00000611, 0, 0, 0, 0, 0, 0,
            ],
        ),
    ];
    for (i, (t, want)) in cases.iter().enumerate() {
        assert_eq!(texture_descriptor(t), *want, "case {i}: the builder's default words");
        assert_eq!(texture_descriptor_lod(t, &TexLod::DEFAULT), *want, "case {i}: the defaults spelled out");
        /* the GL defaults at their bounds change nothing either: max level
         * past the chain, LODs past either end */
        assert_eq!(texture_descriptor_lod(t, &lod(0, 15, -1.0, 15.0)), *want, "case {i}");
    }
}

/* ---- the base level ---- */

/// A linear RGBA8 64x64 chain of seven levels from base level 2: the image
/// is stated from level 2 (the STRIDE word has no baselevel), its levels
/// counted from there, the sampler's range the five levels it walks.
#[test]
fn a_linear_image_is_stated_from_its_base_level() {
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 2, 7, (1, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(2, 1000, -1000.0, 1000.0)));
    assert_eq!(texaddr(f.iw1), BASE + mesa_level_offset(64, 64, 4, 2), "texaddr at level 2");
    assert_eq!(mesa_level_offset(64, 64, 4, 2), 64 * 64 * 4 + 32 * 32 * 4);
    assert_eq!((f.width, f.height), (16, 16), "IMAGE_WORD0 extent: level 2's");
    assert_eq!(bits(f.iw1, 0, 14) + 1, 16, "STRIDE: u_minify(physical_width, 2)");
    assert_eq!(bits(f.iw1, 60, 63), 5, "num_mip_levels: levels 2..6");
    assert_eq!(bits(f.iw1, 15, 15), 1, "mipmaps_present");
    assert_eq!(bits(f.iw1, 59, 59), 1, "alpha_msb as before");
    assert_eq!((f.minlod, f.maxlod), (0, 4 * 64), "LOD 0..4 from the base");
    assert_eq!(f.layer_size, 16 * 16 * 4, "image meta layer size: the base level's");

    /* without a mipmap filter: the base level alone */
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 0, 7, (1, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(3, 1000, -1000.0, 1000.0)));
    assert_eq!(texaddr(f.iw1), BASE + mesa_level_offset(64, 64, 4, 3));
    assert_eq!((f.width, f.height, bits(f.iw1, 60, 63), bits(f.iw1, 15, 15)), (8, 8, 1, 0));
    assert_eq!(f.maxlod, 0);
}

/// An 800x600 chain (laid out as the 1024x1024 one) from base level 1:
/// level 1's offset in the power-of-two chain, its floor-halved extent.
#[test]
fn a_npot_chain_moves_by_the_power_of_two_level_size() {
    let mut t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 800, 600, 2, 10, (1, 1));
    t.stride_texels = 1024;
    let f = fields(&texture_descriptor_lod(&t, &lod(1, 1000, -1000.0, 1000.0)));
    assert_eq!(texaddr(f.iw1), BASE + mesa_level_offset(1024, 1024, 4, 1));
    assert_eq!((f.width, f.height, bits(f.iw1, 0, 14) + 1), (400, 300, 512));
    assert_eq!((bits(f.iw1, 60, 63), f.maxlod), (9, 8 * 64));

    /* 513 rows: level 1 has 256, whose power of two is not the 512 rows it
     * sits in, so the TPU would walk levels 2.. from the wrong rows: the
     * base level alone is described */
    let mut t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 300, 513, 2, 10, (1, 1));
    t.stride_texels = 512;
    let f = fields(&texture_descriptor_lod(&t, &lod(1, 1000, -1000.0, 1000.0)));
    assert_eq!(texaddr(f.iw1), BASE + mesa_level_offset(512, 1024, 4, 1));
    assert_eq!((f.width, f.height, bits(f.iw1, 0, 14) + 1), (150, 256, 256));
    assert_eq!((bits(f.iw1, 60, 63), f.maxlod), (1, 0));
}

/// An ETC2 chain (the twiddled IMAGE_WORD1): baselevel at [60:63], Mesa's
/// level count from it at [0:3], the image address and extent level 0's.
#[test]
fn a_twiddled_image_names_its_base_level() {
    let t = bind(GLES_TEXFMT_ETC2_RGB, GLES_TEXTYPE_2D, 64, 64, 2, 7, (1, 1));
    let w = texture_descriptor_lod(&t, &lod(2, 1000, -1000.0, 1000.0));
    let f = fields(&w);
    assert_eq!(bits(f.iw1, 60, 63), 2, "baselevel");
    assert_eq!(bits(f.iw1, 0, 3), 5, "num_mip_levels: the view's 5 levels");
    assert_eq!(bits(f.iw1, 15, 15), 1, "mipmaps_present");
    assert_eq!(texaddr(f.iw1), BASE);
    assert_eq!((f.width, f.height), (64, 64));
    assert_eq!(f.maxlod, 4 * 64);
    /* the default word with those three fields changed */
    assert_eq!((w[2], w[3]), (0x0000_8005, 0x2060_0004));

    /* one level sampled from level 3: still a chain in memory */
    let t = bind(GLES_TEXFMT_ETC2_RGB, GLES_TEXTYPE_2D, 64, 64, 0, 7, (1, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(3, 1000, -1000.0, 1000.0)));
    assert_eq!((bits(f.iw1, 60, 63), bits(f.iw1, 0, 3), bits(f.iw1, 15, 15)), (3, 1, 1));
}

/// The native cube: baselevel over its copy of the whole chain, whose level
/// count (the face stride) stays the copy's; a one-level copy is level 0
/// alone and keeps baselevel 0.
#[test]
fn the_native_cube_names_its_base_level_over_a_chain_copy() {
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, 64, 64, 2, 7, (1, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(1, 1000, -1000.0, 1000.0)));
    assert_eq!((bits(f.iw1, 60, 63), bits(f.iw1, 0, 3), bits(f.iw1, 4, 14)), (1, 7, 5));
    assert_eq!(f.maxlod, 5 * 64, "levels 1..6");
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, 64, 64, 0, 7, (1, 1));
    let base = texture_descriptor_lod(&t, &TexLod::DEFAULT);
    assert_eq!(texture_descriptor_lod(&t, &lod(1, 1000, -1000.0, 1000.0)), base, "one-level copy");
}

/// The emulated cube is a tall image the shader addresses: its base level
/// is the GL core's to apply, the words stay the default's.
#[test]
fn the_emulated_cube_leaves_the_base_level_to_the_gl_core() {
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_CUBE, 64, 64 * 6, 2, 7, (1, 1));
    assert_eq!(texture_descriptor_lod(&t, &lod(2, 1000, -1000.0, 1000.0)), texture_descriptor(&t));
}

/* ---- GL_TEXTURE_MAX_LEVEL ---- */

#[test]
fn the_max_level_caps_the_levels_walked() {
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 2, 7, (1, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(0, 2, -1000.0, 1000.0)));
    assert_eq!((texaddr(f.iw1), bits(f.iw1, 60, 63), f.maxlod), (BASE, 3, 2 * 64));
    let f = fields(&texture_descriptor_lod(&t, &lod(1, 4, -1000.0, 1000.0)));
    assert_eq!(texaddr(f.iw1), BASE + mesa_level_offset(64, 64, 4, 1));
    assert_eq!((bits(f.iw1, 60, 63), f.maxlod), (4, 3 * 64), "levels 1..4");
    /* max level == base level: one level, no mipmap walk */
    let f = fields(&texture_descriptor_lod(&t, &lod(2, 2, -1000.0, 1000.0)));
    assert_eq!((bits(f.iw1, 60, 63), bits(f.iw1, 15, 15), f.maxlod), (1, 0, 0));
}

/* ---- GL_TEXTURE_MIN_LOD and GL_TEXTURE_MAX_LOD ---- */

#[test]
fn min_and_max_lod_clamp_as_mesas_sampler() {
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 2, 7, (1, 1));
    let chain = 6 * 64;
    for (lo, hi) in [(1.5f32, 2.25f32), (0.0, 1.0), (0.3, 5.9), (-3.0, 4.0), (2.0, 2.0)] {
        let f = fields(&texture_descriptor_lod(&t, &lod(0, 1000, lo, hi)));
        let maxlod = chain.min(mesa_lod(hi));
        assert_eq!((f.minlod, f.maxlod), (mesa_lod(lo).min(maxlod), maxlod), "LOD {lo}..{hi}");
    }
    assert_eq!(mesa_lod(1.5), 96);
    assert_eq!(mesa_lod(2.25), 144);
    /* a min LOD past the chain: the TPU's range collapses to its end */
    let f = fields(&texture_descriptor_lod(&t, &lod(0, 1000, 20.0, 30.0)));
    assert_eq!((f.minlod, f.maxlod), (chain, chain));
    /* max LOD at most 0: always magnified */
    let f = fields(&texture_descriptor_lod(&t, &lod(0, 1000, -1000.0, -1.0)));
    assert_eq!((f.minlod, f.maxlod), (0, 0));
    /* relative to the base level: base 2 walks 4 more levels */
    let f = fields(&texture_descriptor_lod(&t, &lod(2, 1000, 0.5, 1000.0)));
    assert_eq!((f.minlod, f.maxlod), (mesa_lod(0.5), 4 * 64));

    /* one level with min != mag: the LOD 1.0 the filter choice needs, under
     * MAX_LOD and MIN_LOD as well */
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, 64, 64, 0, 1, (0, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(0, 1000, 0.25, 0.5)));
    assert_eq!((f.minlod, f.maxlod), (mesa_lod(0.25), mesa_lod(0.5)));
    let f = fields(&texture_descriptor_lod(&t, &lod(0, 1000, -1000.0, 0.0)));
    assert_eq!(f.maxlod, 0, "max LOD 0: GL magnifies");

    /* the native cube's sampler takes them too */
    let t = bind(GLES_TEXFMT_RGBA8, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, 64, 64, 2, 7, (1, 1));
    let f = fields(&texture_descriptor_lod(&t, &lod(0, 1000, 1.0, 3.5)));
    assert_eq!((f.minlod, f.maxlod), (mesa_lod(1.0), mesa_lod(3.5)));
}
