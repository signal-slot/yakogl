// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`),
//! against docs/notes/pvr-3d-native.md: a sampler3D compiles to one `smp`
//! with `dmn` 3d over the three coordinates (the TPU picks the level,
//! filters across slices and wraps every axis), the record says native 3D
//! (`native_cube` bit 2), `textureSize` reads the image word, and with the
//! switch off the grid of slices (two 2D samples mixed, level 0) is what
//! comes out, as before.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_3D, GLES_TEXTYPE_3D_TWIDDLED, GLES_WRAP_CLAMP_TO_EDGE,
    GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_REPEAT,
};
use yakogl::gl::native_vol::{
    chain_levels, copy_bytes, level_extent, level_texel_off, level_texels, physical_extent, twiddle3d,
};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The switch on or off for the closure, and back at its default after it,
/// panic or not.
fn with_native_3d<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_3d(i32::from(on));
    f()
}

/// One shader through the online PowerVR chain: the record (its code kept
/// by `store::begin_keep` for the closure) or the refusal.
fn compile_with<T>(stage: Stage, src: &str, f: impl FnOnce(&ShcShader, &[u8]) -> T) -> Result<T, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a)
        } else {
            compile_vertex(&l.module, &l.reflection, a)
        }
        .map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        Ok(f(&rec, &image))
    })
    .unwrap()
}

/// The `smp` fields of every sample in a program, by name, in program order.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 11] =
        ["fcnorm", "dmn", "chan", "lodm", "pplod", "proj", "nncoords", "soo", "tao", "sbmode", "sno"];
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if ops_for(r, ph).any(|o| o.name == "smp") {
                out.push(NAMES.iter().map(|n| (*n, r.get(n).unwrap_or(0))).collect());
            }
        }
        off += g.total;
    }
    out
}

fn fragment(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\nuniform vec3 g;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nout vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nin vec4 a;\nout vec4 v;\n\
         void main() {{ gl_Position = a; {body} }}\n"
    )
}

/// An `smp` field's enumerated value by name, from the ISA tables
/// (pco_isa.py): `lodm` normal/bias/replace/gradients, `dmn` 1d/2d/3d.
fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

/// One row per form of a sampler3D look-up: (the look-up, lodm, pplod,
/// nncoords). The level word is in the data block for a bias or a replaced
/// level (`pplod`), absent for the implicit one.
const FORMS: &[(&str, &str, u32, u32)] = &[
    ("texture(s, c)", "normal", 0, 0),
    ("texture(s, c, l)", "bias", 1, 0),
    ("textureLod(s, c, l)", "replace", 1, 0),
    ("textureProj(s, vec4(c, 2.0))", "normal", 0, 0),
    ("texelFetch(s, p, il)", "replace", 1, 1),
];

/// Every form compiles, in a fragment shader, to one `smp` with `dmn` 3d,
/// no `tao`, four channels, `fcnorm` for a float sampler and the row's level
/// mode; the record says native 3D (bit 2) and neither cube nor array.
#[test]
fn every_form_samples_the_tpus_3d_texture() {
    for (e, mode, pplod, nncoords) in FORMS {
        let src = fragment("uniform highp sampler3D s;", &format!("o = {e};"));
        let (flag, fields) = with_native_3d(true, || {
            compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image)))
                .unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(flag, 4, "{e}: the record says native 3D");
        assert_eq!(fields.len(), 1, "{e}: one sample, not two slices mixed");
        let want: BTreeMap<&str, u32> = [
            ("fcnorm", 1),
            ("dmn", enum_value("dmn", "3d")),
            ("chan", 4),
            ("lodm", enum_value("lodm", mode)),
            ("pplod", *pplod),
            ("proj", 0),
            ("nncoords", *nncoords),
            ("soo", 0),
            ("tao", 0),
            ("sbmode", 0),
            ("sno", 0),
        ]
        .into_iter()
        .collect();
        assert_eq!(fields[0], want, "{e}");
    }
}

/// A vertex shader samples the same way (textureLod is its only form).
#[test]
fn a_vertex_shader_samples_the_volume_too() {
    let src = vertex("uniform highp sampler3D s;", "v = textureLod(s, c, l);");
    let (flag, fields) = with_native_3d(true, || {
        compile_with(Stage::Vertex, &src, |rec, image| (rec.native_cube, smps(image))).unwrap()
    });
    assert_eq!(flag, 4);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["dmn"], enum_value("dmn", "3d"));
    assert_eq!(fields[0]["lodm"], enum_value("lodm", "replace"));
}

/// With the switch off the volume stays the grid of slices: the two slices
/// around the depth, sampled as 2D images and mixed -- two `smp`, `dmn` 2d,
/// the record clear.
#[test]
fn off_the_volume_stays_the_grid_of_slices() {
    let src = fragment("uniform highp sampler3D s;", "o = texture(s, c);");
    let (flag, fields) = with_native_3d(false, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image))).unwrap()
    });
    assert_eq!(flag, 0);
    assert_eq!(fields.len(), 2, "two slices");
    assert!(fields.iter().all(|f| f["dmn"] == enum_value("dmn", "2d")));
}

/// A dead volume look-up takes its sample and its third coordinate with it:
/// the program compiles and the record says nothing. A program with a cube,
/// an array-free volume and a native cube says both bits (5).
#[test]
fn a_dead_sample_sets_nothing_and_a_cube_beside_a_volume_sets_both_bits() {
    let dead = fragment("uniform highp sampler3D s;", "vec4 t = texture(s, c); o = vec4(c, 1.0);");
    let flag = with_native_3d(true, || compile_with(Stage::Fragment, &dead, |rec, _| rec.native_cube).unwrap());
    assert_eq!(flag, 0, "a dead look-up: compiled, nothing native left");
    let both =
        fragment("uniform highp sampler3D s;\nuniform highp samplerCube k;", "o = texture(s, c) + texture(k, c);");
    let flag = with_native_3d(true, || {
        yakogl::shaderc::pvr_set_native_cube(1);
        let f = compile_with(Stage::Fragment, &both, |rec, _| rec.native_cube);
        yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        f.unwrap()
    });
    assert_eq!(flag, 5, "a native cube (bit 0) and a native volume (bit 2) in one program");
}

/// `textureSize` of a native volume reads the image word: no `smp`, and the
/// program compiles with a level argument. The word is the twiddled copy's
/// (a grid's is a stride image with no depth), so the record asks for that
/// copy (bit 2) even though nothing is sampled.
#[test]
fn texture_size_reads_the_image_word_and_samples_nothing() {
    let src = fragment("uniform highp sampler3D s;", "o = vec4(textureSize(s, il), 1.0);");
    let (flag, n) = with_native_3d(true, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image).len())).unwrap()
    });
    assert_eq!((flag, n), (4, 0));
}

/// The sampler words of a descriptor as the TPU reads them (texstate.xml
/// SAMPLER_WORD0): (minlod, maxlod, mipfilter, addrmode u, v, w).
fn sampler(w: &[u32]) -> (u64, u64, u64, u64, u64, u64) {
    let q = w[8] as u64 | (w[9] as u64) << 32;
    ((q >> 13) & 0x3ff, (q >> 23) & 0x3ff, (q >> 40) & 1, (q >> 41) & 7, (q >> 44) & 7, (q >> 56) & 7)
}

/// A native 3D bind of `w x h x d` (level 0), `levels` levels.
fn volume_bind(w: u32, h: u32, d: u16, levels: u8, mip_filter: u8) -> GlesTextureBind {
    let phys = physical_extent(w, h, u32::from(d));
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_3D_TWIDDLED,
        layers: d,
        base_gpu: 0x80_0000_9000,
        width: w,
        height: h,
        stride_texels: w,
        bytes: copy_bytes(phys, 4, u32::from(levels)),
        linear_filter: 1,
        min_linear: 1,
        mip_filter,
        mip_levels: levels,
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        wrap_r: GLES_WRAP_REPEAT,
        ..GlesTextureBind::ZERO
    }
}

/// (a) A mipmapped NPOT volume (63x57x67: physical 64x64x128, a chain of
/// eight levels to 1x1x1): the levels follow each other at the offsets
/// `native_vol` computes, each texel of each API level (its size halved, at
/// least 1) lands at a distinct twiddled index inside its own level's padded
/// extent, so no level writes into another, and the copy's size is the last
/// level's end.
#[test]
fn an_npot_mipmapped_volume_writes_every_level_at_its_offset() {
    let (w, h, d) = (63u32, 57u32, 67u32);
    let phys = physical_extent(w, h, d);
    assert_eq!(phys, (64, 64, 128));
    let n = chain_levels(phys);
    assert_eq!(n, 8, "128 -> 1");
    /* the API's own chain is shorter than the copy's: 67 -> 1 is seven levels */
    let api = 32 - w.max(h).max(d).leading_zeros();
    assert_eq!(api, 7);
    let mut off = 0;
    for l in 0..n {
        assert_eq!(level_texel_off(phys, l), off, "level {l} follows the one before");
        let e = level_extent(phys, l);
        assert_eq!(e, ((64 >> l).max(1), (64 >> l).max(1), (128 >> l).max(1)));
        off += level_texels(e);
    }
    assert_eq!(copy_bytes(phys, 4, 1), level_texels(phys) * 4, "no mipmap filter: level 0 alone");
    for levels in 2..=n {
        assert_eq!(copy_bytes(phys, 4, levels), off * 4, "{levels} levels: the whole chain");
    }
    for l in 0..api {
        let (lw, lh, ld) = ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1));
        let e = level_extent(phys, l);
        let mut seen = vec![false; level_texels(e) as usize];
        for z in 0..ld {
            for y in 0..lh {
                for x in 0..lw {
                    let i = twiddle3d(x, y, z, e) as usize;
                    assert!(i < seen.len(), "level {l} ({x}, {y}, {z}) = {i} leaves its {} texels", seen.len());
                    assert!(!seen[i], "level {l} ({x}, {y}, {z}) collides at {i}");
                    seen[i] = true;
                }
            }
        }
        let end = (level_texel_off(phys, l) + level_texels(e)) * 4;
        assert!(end <= copy_bytes(phys, 4, api), "level {l} ends inside the copy");
    }
}

/// (b) The descriptor of the NPOT volume carries level 0's API size, the
/// depth, the level count and the LOD range, so LINEAR_MIPMAP_LINEAR walks
/// above level 0: maxlod is 64 per level less one (u4.6), minlod stays 0 and
/// the mip filter bit is set only for a linear mix between levels.
#[test]
fn the_descriptor_carries_the_level_count_and_the_lod_range() {
    let t = volume_bind(63, 57, 67, 7, 2);
    let w = yakogl::pvr::backend::texture_descriptor(&t);
    let iw0 = w[0] as u64 | (w[1] as u64) << 32;
    let iw1 = w[2] as u64 | (w[3] as u64) << 32;
    assert_eq!(iw0 & 7, 2, "TEXTYPE_3D");
    assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (62, 56), "level 0's API size");
    assert_eq!(iw1 & 0xf, 7, "seven levels");
    assert_eq!((iw1 >> 4) & 0x7ff, 66, "depth - 1");
    assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
    let (minlod, maxlod, mip, ..) = sampler(&w);
    assert_eq!((minlod, maxlod, mip), (0, 6 * 64, 1), "LOD 0 to 6, mixing levels");
    for levels in 2u8..=8 {
        let w = yakogl::pvr::backend::texture_descriptor(&volume_bind(63, 57, 67, levels, 2));
        let s = sampler(&w);
        assert_eq!(w[2] & 0xf, u32::from(levels));
        assert_eq!((s.0, s.1, s.2), (0, u64::from(levels - 1) * 64, 1), "{levels} levels");
    }
    /* MIPMAP_NEAREST: the levels are walked but not mixed */
    let n = sampler(&yakogl::pvr::backend::texture_descriptor(&volume_bind(63, 57, 67, 7, 1)));
    assert_eq!((n.1, n.2), (6 * 64, 0));
    /* no mipmap filter: one level, LOD 0 */
    let w1 = yakogl::pvr::backend::texture_descriptor(&volume_bind(63, 57, 67, 1, 0));
    let s1 = sampler(&w1);
    assert_eq!((w1[2] & 0xf, (w1[3] >> 15) & 1, s1.1, s1.2), (1, 0, 0, 0));
}

/// (c) REPEAT and MIRRORED_REPEAT reach the sampler on S, T and R: each axis
/// has its own mode, none is clamped, and they do not leak into each other.
#[test]
fn repeat_and_mirrored_repeat_reach_the_sampler_on_every_axis() {
    const REPEAT: u64 = 0;
    const FLIP: u64 = 1;
    const CLAMP: u64 = 2;
    let modes = [(GLES_WRAP_REPEAT, REPEAT), (GLES_WRAP_MIRRORED_REPEAT, FLIP), (GLES_WRAP_CLAMP_TO_EDGE, CLAMP)];
    for (ms, es) in modes {
        for (mt, et) in modes {
            for (mr, er) in modes {
                let t = GlesTextureBind { wrap_s: ms, wrap_t: mt, wrap_r: mr, ..volume_bind(63, 57, 67, 7, 2) };
                let s = sampler(&yakogl::pvr::backend::texture_descriptor(&t));
                assert_eq!((s.3, s.4, s.5), (es, et, er), "s={ms} t={mt} r={mr}");
            }
        }
    }
}

/// (d) With the switch off the texture is a grid of slices: the descriptor is
/// the stride 2D image it always was -- not TEXTYPE_3D, one level, the third
/// axis clamped whatever WRAP_R says (the grid's depth is the shader's).
#[test]
fn off_the_descriptor_is_the_grids_stride_image() {
    let t = GlesTextureBind {
        type_: GLES_TEXTYPE_3D,
        height: 8 * 4,
        bytes: 8 * 8 * 4 * 4 * 4,
        ..volume_bind(8, 8, 4, 1, 0)
    };
    let w = yakogl::pvr::backend::texture_descriptor(&t);
    assert_eq!(w[0] & 7, 4, "a STRIDE image, not TEXTYPE_3D");
    let (_, maxlod, mip, _, _, wr) = sampler(&w);
    assert_eq!((maxlod, mip), (0, 0), "level 0 only");
    assert_eq!(wr, 2, "the third axis stays CLAMP_TO_EDGE under REPEAT");
    /* the same bind, native: REPEAT on R is the TPU's */
    let n =
        sampler(&yakogl::pvr::backend::texture_descriptor(&GlesTextureBind { type_: GLES_TEXTYPE_3D_TWIDDLED, ..t }));
    assert_eq!(n.5, 0);
}
