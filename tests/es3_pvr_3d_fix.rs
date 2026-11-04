// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`): the
//! fixes to the native path (docs/notes/pvr-3d-native.md), each against what
//! it fixes.
//!
//! * The order of a level's texels is `usclib_twiddle3d` over the level's
//!   API extent, not over the physical extent halved: Mesa writes a 3D
//!   level through the PBE as a surface of `extent >> level`
//!   (`pvr_blit.c:182-216`) and addresses a 3D storage texel with the API
//!   extent at the level (`pco_nir_tex.c:1162-1181`). The two orders differ
//!   for a non-power-of-two volume whose short axis is just over a power of
//!   two (9x32x4 at level 1); for 63x57x67, the note's example, they agree.
//! * Every 3D look-up form is native with the switch on: the draw binds the
//!   twiddled copy for every 3D binding of a program whose record says bit
//!   2, so a look-up left on the grid arms (an integer sampler) would read
//!   the copy as a grid; `textureGrad` and `texelFetchOffset` on a 3D
//!   sampler were refused either way.
//! * The implicit level of a smooth varying takes exact plane gradients, as
//!   the native cube's does (`pvr_proj_lod` 2): the TPU's own level is one
//!   per 2x2 quad.
//! * With the switch off nothing moved: the grid arms, as before.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d_fix
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::gl::native_vol::{
    chain_levels, level_extent, level_texel_off, level_texels, physical_extent, twiddle3d, twiddle_extent,
};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

/* ---- the layout --------------------------------------------------------- */

/// A level's twiddle extent is level 0's API extent halved: 63x57x67 at
/// level 2 is 15x14x16 where the physical level is 16x16x32; for a
/// power-of-two volume the two are the same at every level.
#[test]
fn a_levels_twiddle_extent_is_the_api_extent_halved() {
    let api = (63, 57, 67);
    let phys = physical_extent(api.0, api.1, api.2);
    assert_eq!(phys, (64, 64, 128));
    assert_eq!(twiddle_extent(api, 0), api);
    assert_eq!(twiddle_extent(api, 2), (15, 14, 16));
    assert_eq!(level_extent(phys, 2), (16, 16, 32));
    assert_eq!(twiddle_extent(api, 9), (1, 1, 1));
    for l in 0..chain_levels((64, 32, 8)) {
        assert_eq!(twiddle_extent((64, 32, 8), l), level_extent((64, 32, 8), l), "a power of two: level {l}");
    }
}

/// Where the two orders part. The cube bits interleave y, x, z a round,
/// an axis dropping out when its cubes are used up; the physical extent
/// keeps an axis one cube longer than the API extent when that axis is
/// just over a power of two (9: physical 16, halved 8, where 9 halved is
/// 4). At 63x57x67 the extra bit is z's last one, past every texel's, so
/// the orders agree; at 9x32x4, level 1 (4x16x2, physical 8x16x2), x's
/// extra bit comes before y's second and moves every texel from y = 8 on.
#[test]
fn an_npot_level_is_ordered_by_its_api_cube_count() {
    let api = (9, 32, 4);
    let phys = physical_extent(api.0, api.1, api.2);
    assert_eq!((twiddle_extent(api, 1), level_extent(phys, 1)), ((4, 16, 2), (8, 16, 2)));
    /* API: one cube across, four down: y's two bits are the cube index */
    assert_eq!(twiddle3d(0, 4, 0, twiddle_extent(api, 1)), 64);
    assert_eq!(twiddle3d(0, 8, 0, twiddle_extent(api, 1)), 128);
    assert_eq!(twiddle3d(0, 12, 1, twiddle_extent(api, 1)), 3 * 64 + 16);
    /* physical: y0, x0, y1 -- texel (0, 8) is cube 4, not 2 */
    assert_eq!(twiddle3d(0, 4, 0, level_extent(phys, 1)), 64);
    assert_eq!(twiddle3d(0, 8, 0, level_extent(phys, 1)), 256);
    /* 63x57x67: the same index either way, at every level */
    let api = (63, 57, 67);
    let phys = physical_extent(api.0, api.1, api.2);
    for l in 0..chain_levels(phys) {
        let (w, h, d) = twiddle_extent(api, l);
        for (x, y, z) in [(w - 1, 0, 0), (0, h - 1, 0), (0, 0, d - 1), (w - 1, h - 1, d - 1), (w / 2, h / 3, d / 2)] {
            assert_eq!(
                twiddle3d(x, y, z, (w, h, d)),
                twiddle3d(x, y, z, level_extent(phys, l)),
                "level {l} ({x}, {y}, {z})"
            );
        }
    }
}

/// Every level of a non-power-of-two chain fits the room the physical
/// chain gives it, and no two of its texels meet: the copy's levels never
/// overlap.
#[test]
fn every_npot_level_fits_its_place_and_is_one_to_one() {
    for api in [(63u32, 57u32, 67u32), (9, 32, 4), (9, 5, 17), (3, 7, 5), (33, 1, 2)] {
        let phys = physical_extent(api.0, api.1, api.2);
        for l in 0..chain_levels(phys) {
            let (w, h, d) = twiddle_extent(api, l);
            let room = level_texels(level_extent(phys, l));
            assert_eq!(level_texel_off(phys, l + 1) - level_texel_off(phys, l), room);
            let mut seen = vec![false; room as usize];
            for z in 0..d {
                for y in 0..h {
                    for x in 0..w {
                        let i = twiddle3d(x, y, z, (w, h, d));
                        assert!(i < room, "{api:?} level {l} ({x}, {y}, {z}) at {i} of {room}");
                        assert!(!seen[i as usize], "{api:?} level {l} ({x}, {y}, {z}) collides at {i}");
                        seen[i as usize] = true;
                    }
                }
            }
        }
    }
}

/* ---- the shaders -------------------------------------------------------- */

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
fn smps(image: &[u8]) -> Vec<Smp> {
    const NAMES: [&str; 8] = ["fcnorm", "dmn", "chan", "lodm", "pplod", "proj", "nncoords", "tao"];
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

/// An `smp` field's enumerated value by name, from the ISA tables.
fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

/// A fragment shader over uniforms (no varying: the TPU's own level).
fn fragment(decl: &str, out: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\nuniform vec3 g;\n\
         uniform vec3 h;\nuniform float l;\nuniform ivec3 p;\nuniform int il;\nout {out} o;\nvoid main() {{ {body} }}\n"
    )
}

/// One `smp`'s fields, by name.
type Smp = BTreeMap<&'static str, u32>;

/// The record's bit and the `smp` fields of one fragment shader.
fn frag(on: bool, src: &str) -> Result<(u8, Vec<Smp>), String> {
    with_native_3d(on, || compile_with(Stage::Fragment, src, |rec, image| (rec.native_cube, smps(image))))
}

/// `textureGrad` on a sampler3D (and its projective form): one `smp` with
/// `dmn` 3d and `lodm` gradients, native; with the switch off it is refused
/// as before (the grid has no form for it).
#[test]
fn texture_grad_samples_the_volume_with_its_gradients() {
    for e in ["textureGrad(s, c, g, h)", "textureProjGrad(s, vec4(c, 2.0), g, h)"] {
        let src = fragment("uniform highp sampler3D s;", "vec4", &format!("o = {e};"));
        let (flag, f) = frag(true, &src).unwrap_or_else(|m| panic!("{e}: {m}"));
        assert_eq!(flag, 4, "{e}");
        assert_eq!(f.len(), 1, "{e}");
        assert_eq!(f[0]["dmn"], enum_value("dmn", "3d"), "{e}");
        assert_eq!(f[0]["lodm"], enum_value("lodm", "gradients"), "{e}");
        assert_eq!(f[0]["nncoords"], 0, "{e}");
        assert!(frag(false, &src).is_err(), "{e}: refused off, as before");
    }
}

/// An integer volume is native too: `fcnorm` off (the texel's integers),
/// `dmn` 3d; with the switch off it stays the grid's nearest slice, a 2D
/// sample.
#[test]
fn an_integer_volume_is_native_too() {
    for (decl, out) in [("uniform highp isampler3D s;", "ivec4"), ("uniform highp usampler3D s;", "uvec4")] {
        for e in ["texture(s, c)", "textureLod(s, c, l)", "texelFetch(s, p, il)"] {
            let src = fragment(decl, out, &format!("o = {e};"));
            let (flag, f) = frag(true, &src).unwrap_or_else(|m| panic!("{decl} {e}: {m}"));
            assert_eq!(flag, 4, "{decl} {e}");
            assert_eq!(f.len(), 1, "{decl} {e}");
            assert_eq!((f[0]["dmn"], f[0]["fcnorm"]), (enum_value("dmn", "3d"), 0), "{decl} {e}");
            let (flag, f) = frag(false, &src).unwrap_or_else(|m| panic!("{decl} {e} off: {m}"));
            assert_eq!(flag, 0, "{decl} {e} off");
            assert!(f.iter().all(|f| f["dmn"] == enum_value("dmn", "2d")), "{decl} {e} off: the grid");
        }
    }
}

/// A program with a float and an integer volume: both samples native, so
/// the copy the draw binds for both is read as the TPU's 3D image by both.
#[test]
fn a_float_and_an_integer_volume_in_one_program_are_both_native() {
    let src = fragment(
        "uniform highp sampler3D s;\nuniform highp isampler3D t;",
        "vec4",
        "o = texture(s, c) + vec4(texture(t, c));",
    );
    let (flag, f) = frag(true, &src).unwrap();
    assert_eq!(flag, 4);
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|f| f["dmn"] == enum_value("dmn", "3d")), "{f:?}");
}

/// texelFetchOffset on a native volume: the offset added to the integer
/// coordinate, one `smp` with `nncoords` and the level replaced; refused
/// with the switch off, as before.
#[test]
fn texel_fetch_offset_adds_the_offset_to_the_coordinate() {
    let src = fragment("uniform highp sampler3D s;", "vec4", "o = texelFetchOffset(s, p, il, ivec3(1, -2, 3));");
    let (flag, f) = frag(true, &src).unwrap();
    assert_eq!(flag, 4);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0]["dmn"], enum_value("dmn", "3d"));
    assert_eq!(f[0]["nncoords"], 1);
    assert_eq!(f[0]["lodm"], enum_value("lodm", "replace"));
    assert!(frag(false, &src).is_err(), "refused off, as before");
    /* the offset is in the program: not the program without it */
    let plain = fragment("uniform highp sampler3D s;", "vec4", "o = texelFetch(s, p, il);");
    let a = with_native_3d(true, || compile_with(Stage::Fragment, &src, |_, i| i.to_vec())).unwrap();
    let b = with_native_3d(true, || compile_with(Stage::Fragment, &plain, |_, i| i.to_vec())).unwrap();
    assert_ne!(a, b);
}

/// The implicit level of a smooth varying: exact plane gradients (`lodm`
/// gradients), as a native cube takes them, with `pvr_proj_lod` at its
/// default 2; at 1 (2D only) and 0 the TPU's own (`lodm` normal).
#[test]
fn a_varying_volume_look_up_takes_exact_gradients() {
    let src = "#version 300 es\nprecision highp float;\nuniform highp sampler3D s;\nin vec3 v;\nout vec4 o;\n\
               void main() { o = texture(s, v); }\n";
    let lodm = |mode: i32| {
        let r = with_native_3d(true, || {
            yakogl::shaderc::pvr_set_proj_lod(mode);
            let r = compile_with(Stage::Fragment, src, |rec, image| (rec.native_cube, smps(image)));
            yakogl::shaderc::pvr_set_proj_lod(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT);
            r
        });
        let (flag, f) = r.unwrap_or_else(|m| panic!("pvr_proj_lod {mode}: {m}"));
        assert_eq!((flag, f.len()), (4, 1), "pvr_proj_lod {mode}");
        assert_eq!(f[0]["dmn"], enum_value("dmn", "3d"));
        f[0]["lodm"]
    };
    assert_eq!(lodm(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT), enum_value("lodm", "gradients"));
    assert_eq!(lodm(1), enum_value("lodm", "normal"));
    assert_eq!(lodm(0), enum_value("lodm", "normal"));
    /* off: the grid's two slices, each at the TPU's own level, as before */
    let (flag, f) =
        with_native_3d(false, || compile_with(Stage::Fragment, src, |rec, image| (rec.native_cube, smps(image))))
            .unwrap();
    assert_eq!((flag, f.len()), (0, 2));
    assert!(f.iter().all(|f| f["lodm"] == enum_value("lodm", "normal") && f["dmn"] == enum_value("dmn", "2d")));
}

/// A vertex shader's explicit level and fetch on a native volume.
#[test]
fn a_vertex_shader_fetches_from_the_volume_too() {
    let src = "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp sampler3D s;\n\
               uniform highp isampler3D t;\nuniform ivec3 p;\nuniform vec3 c;\nin vec4 a;\nout vec4 v;\n\
               void main() { gl_Position = a; v = texelFetch(s, p, 1) + vec4(textureLod(t, c, 0.0)); }\n";
    let (flag, f) =
        with_native_3d(true, || compile_with(Stage::Vertex, src, |rec, image| (rec.native_cube, smps(image)))).unwrap();
    assert_eq!((flag, f.len()), (4, 2));
    assert!(f.iter().all(|f| f["dmn"] == enum_value("dmn", "3d")), "{f:?}");
}

/// With the switch off every 3D form compiles to the bytes it compiled to
/// before the fixes: the grid arms only (two 2D samples mixed, or one 2D
/// fetch), the record clear; and the forms the grid has none for are still
/// refused. (Byte for byte against the previous compiler: the host-3d
/// item's report.)
#[test]
fn off_every_form_keeps_the_grid() {
    let grid: &[(&str, &str, &str, usize)] = &[
        ("uniform highp sampler3D s;", "vec4", "o = texture(s, c);", 2),
        ("uniform highp sampler3D s;", "vec4", "o = texture(s, c, l);", 2),
        ("uniform highp sampler3D s;", "vec4", "o = textureLod(s, c, l);", 2),
        ("uniform highp sampler3D s;", "vec4", "o = textureProj(s, vec4(c, 2.0));", 2),
        ("uniform highp sampler3D s;", "vec4", "o = texelFetch(s, p, il);", 1),
        ("uniform highp isampler3D s;", "ivec4", "o = texture(s, c);", 1),
    ];
    for (decl, out, body, n) in grid {
        let src = fragment(decl, out, body);
        let (flag, f) = frag(false, &src).unwrap_or_else(|m| panic!("{body}: {m}"));
        assert_eq!((flag, f.len()), (0, *n), "{body}");
        assert!(f.iter().all(|f| f["dmn"] == enum_value("dmn", "2d")), "{body}");
    }
    for body in ["o = textureOffset(s, c, ivec3(1));", "o = textureGrad(s, c, g, h);"] {
        let src = fragment("uniform highp sampler3D s;", "vec4", body);
        assert!(frag(false, &src).is_err(), "{body}: refused, as before");
    }
}
