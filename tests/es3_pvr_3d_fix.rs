// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`,
//! docs/notes/pvr-3d-native.md): the fixes after its first host review.
//!
//! - A level of a NPOT volume is twiddled over its API extent halved (the
//!   PBE's and the image atomics' way of addressing a 3D twiddled level,
//!   `native_vol::twiddle_extent`), inside the physical chain's level.
//! - Every return type samples natively, so a program with a float and an
//!   integer sampler3D never reads the twiddled copy as a grid; and
//!   `textureGrad` is pco's txd over three axes.
//! - `texelFetch` samples at the texel's centre on all three axes, and a
//!   guarded fetch's inactive lanes read texel (0, 0, 0)'s centre.
//! - The 3D image word's `alpha_msb` is the format's.
//! - The switch's environment value on the host (`YAKOGL_PVR_NATIVE_3D`).
//!
//! With the switch off (the default) every form compiles as it did.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d_fix
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_R11G11B10F, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_3D, GLES_TEXTYPE_3D_TWIDDLED,
    GLES_WRAP_REPEAT,
};
use yakogl::gl::native_vol::{
    copy_bytes, level_extent, level_texel_off, level_texels, physical_extent, texel_index, twiddle3d, twiddle_extent,
};
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::sel::{SelDst, SelGroup, SelOp, SelSrc, SmpLod};
use yakogl::shaderc::online::powervr::select::select;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{ir, lower, store};
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

/// A fragment shader through the online PowerVR chain: the record (its code
/// kept by `store::begin_keep` for the closure) or the refusal.
fn compile_with<T>(src: &str, f: impl FnOnce(&ShcShader, &[u8]) -> T) -> Result<T, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment(&l.module, &l.reflection, a).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        Ok(f(&rec, &image))
    })
    .unwrap()
}

/// A fragment shader's selection groups, through `f`.
fn selected<T>(src: &str, f: impl FnOnce(&[SelGroup]) -> T) -> T {
    store::with_scratch(|a| {
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let sel = select(&l.module, a).unwrap_or_else(|e| panic!("select refused: {e:?}"));
        f(sel.groups)
    })
    .unwrap()
}

/// The `smp` fields of every sample in a program, by name, in program order.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 8] = ["fcnorm", "dmn", "chan", "lodm", "pplod", "nncoords", "soo", "tao"];
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

/// An `smp` field's enumerated value by name, from the ISA tables.
fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

/* ------------------------------------------------------------------------- *
 * The layout: a NPOT level's twiddle
 * ------------------------------------------------------------------------- */

/// A level is twiddled over its API extent, not the physical one halved:
/// 65x17x8's level 1 is 32x8x4 (cubes 8x2x1), laid out in a 64x16x4 level
/// (cubes 16x4x1). Texel (8, 4, 0) is cube (2, 1, 0): over 8x2 cubes the
/// bits go y0, x0, x1, x2 -- cube 0b101 = 5; over 16x4 they would go y0, x0,
/// y1, x1 -- cube 0b1001 = 9. At level 0 and for a power-of-two volume the
/// two extents round to the same cubes, so nothing else moved.
#[test]
fn a_npot_level_is_twiddled_over_its_api_extent() {
    let api = (65u32, 17u32, 8u32);
    let phys = physical_extent(api.0, api.1, api.2);
    assert_eq!(phys, (128, 32, 8));
    assert_eq!(twiddle_extent(65, 17, 8, 1), (32, 8, 4));
    assert_eq!(level_extent(phys, 1), (64, 16, 4), "where level 1 starts and what it takes");
    let base = level_texels(phys);
    assert_eq!(level_texel_off(phys, 1), base);
    assert_eq!(texel_index(api, 1, 8, 4, 0), base + 5 * 64, "cube 5 of the API extent's interleave");
    assert_eq!(base + twiddle3d(8, 4, 0, level_extent(phys, 1)), base + 9 * 64, "not the physical one's 9");
    /* level 0, and power-of-two volumes at every level: the same index */
    for (x, y, z) in [(64u32, 16u32, 7u32), (3, 9, 2), (0, 0, 0)] {
        assert_eq!(texel_index(api, 0, x, y, z), twiddle3d(x, y, z, phys));
    }
    for pot in [(4u32, 4u32, 4u32), (16, 8, 32), (64, 64, 64)] {
        for l in 0..4 {
            assert_eq!(twiddle_extent(pot.0, pot.1, pot.2, l), level_extent(pot, l), "{pot:?} level {l}");
        }
    }
}

/// Every texel of every level of NPOT volumes (dEQP's 3D sizes among them)
/// lands inside its own level of the physical chain, at an index no other
/// texel of the volume takes, and the chain ends inside the copy.
#[test]
fn every_level_of_a_npot_volume_stays_inside_its_own_level() {
    for api in [(65u32, 17u32, 5u32), (63, 57, 67), (3, 7, 5), (33, 9, 17), (5, 5, 5), (129, 3, 33)] {
        let phys = physical_extent(api.0, api.1, api.2);
        let levels = 32 - api.0.max(api.1).max(api.2).leading_zeros();
        let mut seen = std::collections::HashSet::new();
        for l in 0..levels {
            let (lw, lh, ld) = twiddle_extent(api.0, api.1, api.2, l);
            let lo = level_texel_off(phys, l);
            let hi = lo + level_texels(level_extent(phys, l));
            for z in 0..ld {
                for y in 0..lh {
                    for x in 0..lw {
                        let i = texel_index(api, l, x, y, z);
                        assert!((lo..hi).contains(&i), "{api:?} level {l} ({x}, {y}, {z}) = {i} leaves {lo}..{hi}");
                        assert!(seen.insert(i), "{api:?} level {l} ({x}, {y}, {z}) collides at {i}");
                    }
                }
            }
            assert!(hi * 4 <= copy_bytes(phys, 4, levels), "{api:?} level {l} ends inside the copy");
        }
    }
}

/// The PBE writes a 3D twiddled level sized `1 << log2ceil(level width)`
/// per axis (`PBESTATE_REG_WORD1.size_x`, from the view's minified extent):
/// the extent `twiddle3d` rounds a level's API extent to, at least a cube.
#[test]
fn the_twiddle_extent_is_the_pbes_size_of_the_level() {
    let log2ceil = |n: u32| 32 - (n.max(1) - 1).leading_zeros();
    for (w, h, d) in [(65u32, 17u32, 5u32), (63, 57, 67), (100, 3, 9)] {
        for l in 0..7 {
            let (lw, lh, ld) = twiddle_extent(w, h, d, l);
            let pbe = (1 << log2ceil(lw), 1 << log2ceil(lh), 1 << log2ceil(ld));
            assert_eq!(pbe, (lw.next_power_of_two(), lh.next_power_of_two(), ld.next_power_of_two()));
            assert_eq!((lw, lh, ld), ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1)), "u_minify");
        }
    }
}

/* ------------------------------------------------------------------------- *
 * The descriptor
 * ------------------------------------------------------------------------- */

fn volume_bind(format: u8, w: u32, h: u32, d: u16, levels: u8) -> GlesTextureBind {
    GlesTextureBind {
        format,
        type_: GLES_TEXTYPE_3D_TWIDDLED,
        layers: d,
        base_gpu: 0x80_0000_9000,
        width: w,
        height: h,
        stride_texels: w,
        bytes: copy_bytes(physical_extent(w, h, u32::from(d)), 4, u32::from(levels)),
        linear_filter: 1,
        min_linear: 1,
        mip_filter: if levels > 1 { 2 } else { 0 },
        mip_levels: levels,
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        wrap_r: GLES_WRAP_REPEAT,
        ..GlesTextureBind::ZERO
    }
}

/// `alpha_msb` (IMAGE_WORD1 bit 59) of the TPU's 3D image is the format's,
/// as its STRIDE word's is: set for RGBA8, clear for R11G11B10F (alpha the
/// constant 1, Mesa's `vk_format_is_alpha_on_msb` false). It was always set.
#[test]
fn the_3d_image_words_alpha_msb_is_the_formats() {
    let msb = |t: &GlesTextureBind| {
        let w = yakogl::pvr::backend::texture_descriptor(t);
        (u64::from(w[2]) | u64::from(w[3]) << 32) >> 59 & 1
    };
    assert_eq!(msb(&volume_bind(GLES_TEXFMT_RGBA8, 8, 8, 8, 4)), 1);
    let rgb = volume_bind(GLES_TEXFMT_R11G11B10F, 8, 8, 8, 4);
    assert_eq!(msb(&rgb), 0, "R11G11B10F has no alpha on the MSB");
    /* the grid's stride word of the same format says the same */
    let grid = GlesTextureBind { type_: GLES_TEXTYPE_3D, height: 64, mip_levels: 1, mip_filter: 0, ..rgb };
    assert_eq!(msb(&grid), 0);
    /* the rest of the word as before: the levels, the depth, mipmaps_present */
    let w = yakogl::pvr::backend::texture_descriptor(&rgb);
    assert_eq!((w[2] & 0xf, (w[2] >> 4) & 0x7ff, (w[2] >> 15) & 1), (4, 7, 1));
}

/* ------------------------------------------------------------------------- *
 * The shader forms
 * ------------------------------------------------------------------------- */

/// An isampler3D and a usampler3D sample the TPU's 3D texture too: one `smp`
/// `dmn` 3d without `fcnorm`, in each form, and the record says bit 2.
#[test]
fn integer_volumes_sample_natively() {
    let forms: &[(&str, &str, u32)] = &[
        ("texture(s, c)", "normal", 0),
        ("textureLod(s, c, l)", "replace", 0),
        ("texelFetch(s, p, il)", "replace", 1),
    ];
    for ty in ["isampler3D", "usampler3D"] {
        for (e, mode, nncoords) in forms {
            let src = fragment(&format!("uniform highp {ty} s;"), &format!("o = vec4({e});"));
            let (flag, f) = with_native_3d(true, || {
                compile_with(&src, |rec, image| (rec.native_cube, smps(image)))
                    .unwrap_or_else(|m| panic!("{ty} {e}: {m}"))
            });
            assert_eq!(flag, 4, "{ty} {e}: the record says native 3D");
            assert_eq!(f.len(), 1, "{ty} {e}");
            assert_eq!(
                (f[0]["dmn"], f[0]["fcnorm"], f[0]["lodm"], f[0]["nncoords"]),
                (enum_value("dmn", "3d"), 0, enum_value("lodm", mode), *nncoords),
                "{ty} {e}"
            );
        }
    }
}

/// A float and an integer volume in one program: the draw binds the twiddled
/// copy for both (the record's bit 2 is per program), so both must sample
/// it -- two `smp` `dmn` 3d. With the switch off both stay on the grid
/// (`dmn` 2d) and the record is clear, as before.
#[test]
fn a_float_and_an_integer_volume_in_one_program_both_sample_the_copy() {
    let src = fragment(
        "uniform highp sampler3D s;\nuniform highp isampler3D t;",
        "o = texture(s, c) + vec4(texelFetch(t, p, il));",
    );
    let (flag, f) = with_native_3d(true, || compile_with(&src, |rec, image| (rec.native_cube, smps(image))).unwrap());
    assert_eq!(flag, 4);
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|s| s["dmn"] == enum_value("dmn", "3d")), "{f:?}");
    assert_eq!((f[0]["fcnorm"] + f[1]["fcnorm"]), 1, "one float sample, one integer");
    let (flag, f) = with_native_3d(false, || compile_with(&src, |rec, image| (rec.native_cube, smps(image))).unwrap());
    assert_eq!(flag, 0);
    assert!(f.iter().all(|s| s["dmn"] == enum_value("dmn", "2d")), "{f:?}");
}

/// `textureGrad` on a native volume: pco's txd, `lodm` gradients, the six
/// gradient words after the three coordinates (d/dx then d/dy of u, v, w),
/// in the selection's order. With the switch off the grid has no form for
/// it and the compile is refused, as before.
#[test]
fn texture_grad_samples_the_volume_with_three_axes_of_gradients() {
    let src =
        fragment("uniform highp sampler3D s;\nuniform vec3 dx;\nuniform vec3 dy;", "o = textureGrad(s, c, dx, dy);");
    let (flag, f) = with_native_3d(true, || compile_with(&src, |rec, image| (rec.native_cube, smps(image))).unwrap());
    assert_eq!(flag, 4);
    assert_eq!(f.len(), 1);
    assert_eq!((f[0]["dmn"], f[0]["lodm"]), (enum_value("dmn", "3d"), enum_value("lodm", "gradients")));
    let grads = with_native_3d(true, || {
        selected(&src, |gs| {
            let at = gs.iter().position(|g| matches!(g.op, SelOp::Sample { lod: SmpLod::Gradients, .. })).unwrap();
            assert!(matches!(gs[at - 1].op, SelOp::SampleZ { vol: true }), "the volume's w right before the sample");
            gs[..at - 1].iter().rev().take_while(|g| g.op == SelOp::SampleGrad).count()
        })
    });
    assert_eq!(grads, 3, "one gradient pair per axis");
    assert!(with_native_3d(false, || compile_with(&src, |_, _| ())).is_err(), "the grid refuses it");
}

/// The group that defines virtual register `v`.
fn def_of(gs: &[SelGroup], v: u32) -> &SelGroup {
    gs.iter()
        .find(|g| matches!(g.dst, Some(SelDst::Virtual(d)) if d.0 == v))
        .unwrap_or_else(|| panic!("nothing defines v{v}"))
}

fn virt(s: Option<SelSrc>) -> u32 {
    match s {
        Some(SelSrc::Virtual { v, .. }) => v.0,
        other => panic!("not a virtual register: {other:?}"),
    }
}

/// Is `s` `float(int) + 0.5`, the texel's centre?
fn is_centre(gs: &[SelGroup], s: Option<SelSrc>) -> bool {
    let g = def_of(gs, virt(s));
    let half = Some(SelSrc::Imm(0.5f32.to_bits()));
    g.op == SelOp::Alu(ir::AluOp::Fadd)
        && g.srcs[1] == half
        && def_of(gs, virt(g.srcs[0])).op == SelOp::Alu(ir::AluOp::I2f)
}

/// `texelFetch` on a native volume samples at the texel's centre on all
/// three axes -- `float(p) + 0.5`, w included, `nncoords` putting all three
/// in texels -- so the binding's own LINEAR filter (no point sampler for a
/// combined image-sampler) returns the texel itself, as on the 2D fetch.
#[test]
fn texel_fetch_samples_the_centre_on_all_three_axes() {
    let src = fragment("uniform highp sampler3D s;", "o = texelFetch(s, p, il);");
    with_native_3d(true, || {
        selected(&src, |gs| {
            let at = gs.iter().position(|g| matches!(g.op, SelOp::Sample { fetch: true, .. })).expect("a fetch");
            let z = &gs[at - 1];
            assert!(matches!(z.op, SelOp::SampleZ { vol: true }));
            assert!(is_centre(gs, gs[at].srcs[0]), "u");
            assert!(is_centre(gs, gs[at].srcs[1]), "v");
            assert!(is_centre(gs, z.srcs[0]), "w");
        })
    });
}

/// A fetch in an `if`: the level is computed in the arm, so the coordinates
/// are guarded too (`guard_sample`) -- and a volume is guarded as a 2D
/// look-up is, not as a cube's direction (1, 0, 0): where the arm is off it
/// reads texel (0, 0, 0)'s centre, 0.5 on every axis.
#[test]
fn a_guarded_fetch_reads_the_first_texels_centre_where_the_arm_is_off() {
    let src = fragment("uniform highp sampler3D s;", "o = vec4(0.0); if (c.x > 0.5) { o = texelFetch(s, p, il + 1); }");
    with_native_3d(true, || {
        selected(&src, |gs| {
            let at = gs.iter().position(|g| matches!(g.op, SelOp::Sample { fetch: true, .. })).expect("a fetch");
            let safe = |s: Option<SelSrc>| -> Option<u32> {
                let g = def_of(gs, virt(s));
                if g.op != SelOp::Alu(ir::AluOp::Bcsel) {
                    return None;
                }
                g.srcs[1..].iter().find_map(|x| match x {
                    Some(SelSrc::Imm(b)) => Some(*b),
                    _ => None,
                })
            };
            let half = Some(0.5f32.to_bits());
            assert_eq!(safe(gs[at].srcs[0]), half, "u");
            assert_eq!(safe(gs[at].srcs[1]), half, "v");
            assert_eq!(safe(gs[at - 1].srcs[0]), half, "w");
        })
    });
    /* and it compiles through the whole chain */
    let n = with_native_3d(true, || compile_with(&src, |_, image| smps(image).len())).unwrap();
    assert_eq!(n, 1);
}

/* ------------------------------------------------------------------------- *
 * The switch
 * ------------------------------------------------------------------------- */

/// The host's environment value: exactly `0` or `1`; anything else leaves
/// the default, which stays OFF.
#[test]
fn the_environment_switch_takes_0_or_1_and_the_default_stays_off() {
    use yakogl::shaderc::switch_setting;
    assert_eq!(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT, 0);
    assert_eq!(switch_setting(Some(c"1")), Some(1));
    assert_eq!(switch_setting(Some(c"0")), Some(0));
    for v in [c"", c"2", c"10", c"on", c" 1"] {
        assert_eq!(switch_setting(Some(v)), None, "{v:?}");
    }
    assert_eq!(switch_setting(None), None);
}

/// With the switch off every form compiles as it did before the fixes: the
/// grid's two slices for a float look-up, one 2D sample for an integer one
/// and for a fetch, the record clear.
#[test]
fn off_every_form_is_the_grids() {
    for (decl, e, n) in [
        ("uniform highp sampler3D s;", "texture(s, c)", 2),
        ("uniform highp sampler3D s;", "texelFetch(s, p, il)", 1),
        ("uniform highp isampler3D s;", "vec4(texture(s, c))", 1),
        ("uniform highp usampler3D s;", "vec4(texelFetch(s, p, il))", 1),
    ] {
        let src = fragment(decl, &format!("o = {e};"));
        let (flag, f) =
            with_native_3d(false, || compile_with(&src, |rec, image| (rec.native_cube, smps(image))).unwrap());
        assert_eq!(flag, 0, "{e}");
        assert_eq!(f.len(), n, "{e}");
        assert!(f.iter().all(|s| s["dmn"] == enum_value("dmn", "2d")), "{e}: {f:?}");
    }
}
