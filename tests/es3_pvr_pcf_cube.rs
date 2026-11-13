// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The compare per texel (PCF) of a `samplerCubeShadow` look-up on the
//! PowerVR online back end (select.rs `pcf_cube_taps`, `cube_face`,
//! `face_direction` and `shadow_compare`), on the native cube
//! (`pvr_native_cube`, the TPU's TEXTYPE_CUBE copy) and on the stacked
//! image. ES 3.0 filters a cube seamlessly and dEQP's reference compares
//! each texel the filter reads before weighting (texref.rs
//! `sample_cube_compare`): the program selects the face and (s, t) of the
//! direction, takes the level(s) the sampler's filter reads (sampler meta
//! dword 13, as for 2D, tests/es3_pvr_pcf.rs), and at each level reads the
//! four taps of the face's bilinear footprint one texel each -- the tap's
//! centre on the face's plane, selected and projected again, so a tap past
//! an edge reads the neighbouring face's edge texel and a tap past a corner
//! is the mean of the other three compares -- by a gather a quarter texel
//! past that texel's centre, whose first tap is the texel.
//!
//! - The arithmetic, on the CPU (`cube_model`, operation for operation as
//!   the program computes it), against texref over the six minification
//!   filters, both magnification filters, directions on every face with
//!   footprints across the edges and the corners, levels of detail over a
//!   chain of four levels and four compare functions.
//! - The program's shape: the plain sample and eight one-channel gathers at
//!   a replaced level, `dmn` 3d on the native cube and 2d on the stacked
//!   image, the quad's derivatives only for a fragment shader's implicit
//!   level or bias.
//! - The compiled programs, in the byte model against `cube_model`: the
//!   gathers' levels and the result, for each filter word, under
//!   `textureGrad`, a bias and the implicit level, on both layouts.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_pcf_cube
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::texref::{self, Compare, CompareFunc, Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::{lower, store};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The native-cube switch set for the closure, and back at its default
/// after it, panic or not.
fn with_native_cube<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        }
    }
    let _back = Back;
    yakogl::shaderc::pvr_set_native_cube(i32::from(on));
    f()
}

/* ---- the filter word and the levels (tests/es3_pvr_pcf.rs) --------------- */

/// Sampler meta dword 13 as src/pvr/backend.rs `shadow_filter_word` writes
/// it for `sp` over `levels` levels.
fn filter_word(sp: &Sampler, levels: u32) -> u32 {
    let mag_linear = sp.mag == Filter::Linear;
    let min_linear = sp.min.within_level() == Filter::Linear;
    let mip = if levels > 1 { sp.min.mipmap() } else { None };
    if !mag_linear && !min_linear && mip != Some(true) {
        return 0;
    }
    let mut m = 1;
    if !mag_linear {
        m |= 2;
    }
    if !min_linear {
        m |= 4;
    }
    match mip {
        Some(false) => m |= 8,
        Some(true) => m |= 16,
        None => {}
    }
    m | (levels - 1).min(15) << 8
}

/// `pcf_levels`: the level(s) at level of detail `lambda` under `word`, the
/// blend weight and whether the filter within the level is NEAREST.
fn pcf_levels(word: u32, lambda: f32) -> ([f32; 2], f32, bool) {
    let bit = |b: u32| word & b != 0;
    let minified = 0.0 < lambda;
    let nearest = if minified { bit(4) } else { bit(2) };
    let top = ((word >> 8) & 15) as f32;
    let clamped = lambda.max(0.0).min(top);
    let first = clamped.floor();
    let fraction = clamped + -first;
    let second = (first + 1.0).min(top);
    let q = -lambda + -0.5;
    let ceil = -q.floor();
    let nearest_level = (ceil + -1.0).max(0.0).min(top);
    let blend = minified && bit(16);
    let one = if bit(8) { nearest_level } else { 0.0 };
    let pick = if bit(16) { first } else { one };
    let d1 = if minified { pick } else { 0.0 };
    let d2 = if blend { second } else { d1 };
    ([d1, d2], if blend { fraction } else { 0.0 }, nearest)
}

/// mix(a, b, t) as the program computes it: t * (b - a) + a.
fn mix(a: f32, b: f32, t: f32) -> f32 {
    t * (b + -a) + a
}

/* ---- the program's cube arithmetic on the CPU ----------------------------- */

/// `cube_face`: the face (0..5, GL's order) and (s, t) of direction `d`,
/// `cube_uv`'s major axis, ties and signs; with `dd` (d/dx and d/dy of the
/// direction) the gradients `[[ds/dx, dt/dx], [ds/dy, dt/dy]]`.
fn cube_face(d: [f32; 3], dd: Option<[[f32; 3]; 2]>) -> (f32, [f32; 2], Option<[[f32; 2]; 2]>) {
    let a = d.map(f32::abs);
    let x_major = a[1] <= a[0] && a[2] <= a[0];
    let y_major = a[2] <= a[1];
    let pos = d.map(|x| 0.0 < x);
    let choose = |x: f32, y: f32, z: f32| {
        if x_major {
            x
        } else if y_major {
            y
        } else {
            z
        }
    };
    let pick = |c: bool, x: f32, y: f32| if c { x } else { y };
    let face = choose(pick(pos[0], 0.0, 1.0), pick(pos[1], 2.0, 3.0), pick(pos[2], 4.0, 5.0));
    let sc = choose(pick(pos[0], -d[2], d[2]), d[0], pick(pos[2], d[0], -d[0]));
    let tc = choose(-d[1], pick(pos[1], d[2], -d[2]), -d[1]);
    let r = 1.0 / choose(a[0], a[1], a[2]);
    let q = [sc * r, tc * r];
    let st = q.map(|q| q * 0.5 + 0.5);
    let grads = dd.map(|dd| {
        let half_r = r * 0.5;
        dd.map(|v| {
            let dsc = choose(pick(pos[0], -v[2], v[2]), v[0], pick(pos[2], v[0], -v[0]));
            let dtc = choose(-v[1], pick(pos[1], v[2], -v[2]), -v[1]);
            let dm = choose(pick(pos[0], v[0], -v[0]), pick(pos[1], v[1], -v[1]), pick(pos[2], v[2], -v[2]));
            [0, 1].map(|axis| (-q[axis] * dm + [dsc, dtc][axis]) * half_r)
        })
    });
    (face, st, grads)
}

/// `face_direction`: the direction of (sc, tc) on face `face`'s plane
/// (texref.rs `face_direction`'s table).
fn face_direction(face: f32, sc: f32, tc: f32) -> [f32; 3] {
    match face as u32 {
        0 => [1.0, -tc, -sc],
        1 => [-1.0, -tc, sc],
        2 => [sc, 1.0, tc],
        3 => [sc, -1.0, -tc],
        4 => [sc, -tc, 1.0],
        _ => [-sc, -tc, -1.0],
    }
}

/// `pcf_lambda` from the gradients of (s, t), level 0's face `n0` wide.
fn lambda_of(g: [[f32; 2]; 2], n0: u32, bias: Option<f32>) -> f32 {
    let nf = n0 as i32 as f32;
    let len = |[du, dv]: [f32; 2]| {
        let (a, b) = (du * nf, dv * nf);
        b * b + a * a
    };
    let l2 = len(g[0]).max(len(g[1])).log2();
    match bias {
        Some(b) => l2 * 0.5 + b,
        None => l2 * 0.5,
    }
}

/// One tap a level's footprint reads: the face and the point a quarter
/// texel past the texel's centre (in [0, 1] on that face) the gather is at,
/// and whether the tap is past a corner of the look-up's face.
#[derive(Clone, Copy, Debug)]
struct Tap {
    face: f32,
    at: [f32; 2],
    corner: bool,
}

/// The four taps at a level whose face is `n` texels wide, for the
/// look-up's face and (s, t), in footprint order (i0, j0), (i1, j0),
/// (i0, j1), (i1, j1).
fn level_taps(face: f32, st: [f32; 2], n: u32) -> [Tap; 4] {
    let nf = n as i32 as f32;
    let rn = 1.0 / nf;
    let last = nf + -1.0;
    let two_rn = rn + rn;
    let centre0 = rn + -1.0;
    let per_axis = |s: f32| {
        let i0 = (s * nf + -0.5).floor();
        [i0, i0 + 1.0].map(|i| (i < 0.0 || nf <= i, i * two_rn + centre0))
    };
    let (u, v) = (per_axis(st[0]), per_axis(st[1]));
    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(a, b)| {
        let p = face_direction(face, u[a].1, v[b].1);
        let (tap_face, tap_st, _) = cube_face(p, None);
        let at = tap_st.map(|x| ((x * nf).floor().max(0.0).min(last) + 0.75) * rn);
        Tap { face: tap_face, at, corner: u[a].0 && v[b].0 }
    })
}

/// What the program's cube arm returns: at the level(s) `word` reads at
/// `lambda`, the taps' compares (`compared(level, tap)`), a corner tap the
/// mean of the other three, weighted by fract(s n - 0.5) at the level's
/// face width `size(level)` (0 or 1 for NEAREST), the two levels mixed.
fn cube_model(
    word: u32,
    lambda: f32,
    face: f32,
    st: [f32; 2],
    size: impl Fn(u32) -> u32,
    compared: impl Fn(u32, &Tap) -> f32,
) -> f32 {
    let ([d1, d2], t, nearest) = pcf_levels(word, lambda);
    let at = |d: f32| {
        let n = size(d as u32);
        let taps = level_taps(face, st, n);
        let mut c = taps.map(|tap| compared(d as u32, &tap));
        let sum = (c[0] + c[1]) + (c[2] + c[3]);
        for (cj, tap) in c.iter_mut().zip(taps) {
            if tap.corner {
                *cj = (sum + -*cj) * (1.0 / 3.0);
            }
        }
        let nf = n as i32 as f32;
        let w = st.map(|x| {
            let s = x * nf + -0.5;
            let fract = s + -s.floor();
            match (nearest, 0.5 <= fract) {
                (false, _) => fract,
                (true, true) => 1.0,
                (true, false) => 0.0,
            }
        });
        mix(mix(c[0], c[1], w[0]), mix(c[2], c[3], w[0]), w[1])
    };
    mix(at(d1), at(d2), t)
}

/* ---- against texref ------------------------------------------------------ */

/// A deterministic depth in [0, 1) per texel.
fn depth(seed: u32, level: u32, face: u32, i: u32, j: u32) -> f32 {
    let mut h = seed ^ 0x9e37_79b9;
    for x in [level, face, i, j] {
        h = (h ^ x).wrapping_mul(0x0100_0193).rotate_left(13);
    }
    (h % 997) as f32 / 997.0
}

/// An 8x8 depth cube with its full chain of four levels, R the depth.
fn cube_chain() -> Texture {
    let mut levels = Vec::new();
    let mut n = 8u32;
    for l in 0..4 {
        let faces =
            (0..6).map(|f| (0..n * n).map(|k| [depth(5, l, f, k % n, k / n), 0.0, 0.0, 1.0]).collect()).collect();
        levels.push(Level { width: n, height: n, faces });
        n = (n / 2).max(1);
    }
    Texture { cube: true, min: MinFilter::Nearest, levels }
}

const MIN_FILTERS: [MinFilter; 6] = [
    MinFilter::Nearest,
    MinFilter::Linear,
    MinFilter::NearestMipmapNearest,
    MinFilter::LinearMipmapNearest,
    MinFilter::NearestMipmapLinear,
    MinFilter::LinearMipmapLinear,
];

/// Points on a face's plane in [-1, 1]: the middle, and close enough to the
/// edges that a footprint at every level crosses them (0.997 is still a
/// face texref names: its tie refusal is within 1e-3).
const PLANE: [f32; 9] = [-0.997, -0.96, -0.81, -0.33, 0.07, 0.41, 0.88, 0.97, 0.995];

/// The program's cube arm, on the CPU with the texels its gathers read,
/// gives texref's seamless percentage-closer result (compare every texel
/// the filter reads, a corner tap the mean of the other three compares,
/// then weight) for every filter pair, level of detail, compare function
/// and direction texref predicts -- on all six faces, the footprints inside
/// a face, across an edge and at a corner; and the words that ask for no
/// per-texel compare are exactly the filters whose one texel is the result.
#[test]
fn the_cube_arm_is_texrefs_seamless_percentage_closer_filter() {
    let t = cube_chain();
    let levels = t.levels.len() as u32;
    let lods = [-0.5f32, 0.0, 0.4, 0.8, 1.3, 1.7, 2.2, 2.9, 3.5, 5.0];
    let funcs = [CompareFunc::Less, CompareFunc::Gequal, CompareFunc::Lequal, CompareFunc::Greater];
    let (mut checked, mut fractions, mut seams, mut corners) = (0usize, 0usize, 0usize, 0usize);
    for min in MIN_FILTERS {
        for mag in [Filter::Nearest, Filter::Linear] {
            let sp = Sampler { mag, min, wrap_s: Wrap::Repeat, wrap_t: Wrap::ClampToEdge };
            let word = filter_word(&sp, levels);
            if word & 1 == 0 {
                /* the filtered depth's compare: one texel is the filter */
                assert!(mag == Filter::Nearest && matches!(min, MinFilter::Nearest | MinFilter::NearestMipmapNearest));
                continue;
            }
            for lod in lods {
                for face in 0..6u32 {
                    for (k, (sc, tc)) in PLANE.iter().flat_map(|a| PLANE.iter().map(move |b| (*a, *b))).enumerate() {
                        let scale = 0.6 + (k % 5) as f32 * 0.7;
                        let [x, y, z] = face_direction(face as f32, sc, tc).map(|c| c * scale);
                        let (pface, st, _) = cube_face([x, y, z], None);
                        assert_eq!(pface, face as f32, "({sc}, {tc}) on face {face}");
                        for func in funcs {
                            let dref = 0.2 + 0.6 * depth(3, k as u32, face, 0, 0);
                            let cmp = Compare { func, dref, clamp_ref: false };
                            let Ok((want, wface)) =
                                texref::sample_cube_compare(&t, &sp, &cmp, x, y, z, texref::Lod::Explicit(lod))
                            else {
                                continue;
                            };
                            assert_eq!(wface.index() as u32, face);
                            let size = |d: u32| t.levels[d as usize].width;
                            let compared = |d: u32, tap: &Tap| {
                                let l = &t.levels[d as usize];
                                let [i, j] = tap.at.map(|a| (a * l.width as f32).floor() as u32);
                                cmp.texel(l.faces[tap.face as usize][(j * l.width + i) as usize])
                            };
                            let got = cube_model(word, lod, pface, st, size, compared);
                            assert!(
                                (got - want).abs() < 1e-4,
                                "{sp:?} word {word:#x}, lod {lod}, face {face} ({sc}, {tc}) x {scale}, \
                                 {func:?} {dref}: the program {got}, texref {want}"
                            );
                            checked += 1;
                            fractions += usize::from(want != 0.0 && want != 1.0);
                            let ([d1, _], _, _) = pcf_levels(word, lod);
                            let taps = level_taps(pface, st, size(d1 as u32));
                            seams += usize::from(taps.iter().any(|tp| tp.face != pface && !tp.corner));
                            corners += usize::from(taps.iter().any(|tp| tp.corner));
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 50_000, "texref predicted {checked} look-ups");
    assert!(fractions > 10_000, "{fractions} of them are fractions: the weights are exercised");
    assert!(seams > 5_000 && corners > 1_000, "{seams} footprints across an edge, {corners} at a corner");
}

/// The texel a tap past an edge reads is the neighbouring face's edge
/// texel along the same row or column (dEQP's getCubeEdgeFaceCoords keeps
/// the index along the shared edge): +X's footprint past s = 1 reads -Z's
/// column 0 in the same row, past t = 0 +Y's last column, at every row and
/// column of an 8-wide face; and the quarter-texel point is inside that
/// texel.
#[test]
fn a_tap_past_an_edge_reads_the_neighbours_edge_texel() {
    let n = 8u32;
    for k in 0..n {
        let c = (k as f32 + 0.5) / n as f32;
        /* +X, s just below 1: the second column is past the edge */
        let taps = level_taps(0.0, [1.0 - 0.1 / n as f32, c], n);
        let past = taps[1];
        assert_eq!(past.face, 5.0, "row {k}: -Z");
        let [i, j] = past.at.map(|a| (a * n as f32).floor() as u32);
        assert_eq!(i, 0, "row {k}: -Z's column next to +X");
        let jj = (taps[0].at[1] * n as f32).floor() as u32;
        assert_eq!(j, jj, "row {k}: the same row on both faces");
        /* +X, t just above 0: the first row is past the edge, into +Y,
         * whose edge with +X is its last column (x = 1), +X's column k
         * along it from +Y's last row (+X's s runs along -z, +Y's t along z) */
        let taps = level_taps(0.0, [c, 0.1 / n as f32], n);
        assert_eq!(taps[0].face, 2.0, "column {k}: +Y");
        let [i, j] = taps[0].at.map(|a| (a * n as f32).floor() as u32);
        assert_eq!((i, j), (n - 1, n - 1 - k), "column {k}: +Y's column next to +X");
        for tap in taps {
            let off = tap.at.map(|a| a * n as f32 - (a * n as f32).floor());
            assert!(off.iter().all(|o| (o - 0.75).abs() < 1e-3), "{tap:?}");
        }
    }
    /* the corner: (s, t) near (1, 0) puts the footprint's (i1, j0) past both */
    let taps = level_taps(0.0, [1.0 - 0.1 / n as f32, 0.1 / n as f32], n);
    assert_eq!(taps.map(|t| t.corner), [false, true, false, false]);
}

/* ---- the compiled program ------------------------------------------------- */

/// One shader through the online PowerVR chain: the closure on the code.
fn compile_with<T>(stage: Stage, src: &str, f: impl FnOnce(&[u8]) -> T) -> Result<T, String> {
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
        Ok(f(&image))
    })
    .unwrap()
}

/// Every instruction of a program, by name, with the `smp` fields a gather
/// is told by.
fn ops(image: &[u8]) -> (Vec<&'static str>, Vec<BTreeMap<&'static str, u32>>) {
    let mut names = Vec::new();
    let mut smps = Vec::new();
    let mut off = 0usize;
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            for o in ops_for(r, ph) {
                names.push(o.name);
                if o.name == "smp" {
                    smps.push(
                        ["sbmode", "chan", "lodm", "pplod", "tao", "dmn"]
                            .iter()
                            .map(|n| (*n, r.get(n).unwrap_or(0)))
                            .collect(),
                    );
                }
            }
        }
        off += g.total;
    }
    (names, smps)
}

fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

const DECLS: &str = "uniform vec4 c;\nuniform vec4 g;\nuniform vec4 h;\nuniform float l;\n";

fn fragment(body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp samplerCubeShadow s;\n{DECLS}\
         out vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp samplerCubeShadow s;\n{DECLS}\
         in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; {body} }}\n"
    )
}

/// The program's shape is the same for every filter (the sampler's, read
/// when the shader runs): the plain sample, then eight gathers -- `sbmode`
/// rawdata, one channel, `lodm` replace -- `dmn` 3d on the native cube and
/// 2d on the stacked image; the quad's derivatives (`fdsx`, `fdsy`) only
/// for a fragment shader's implicit level or bias.
#[test]
fn a_cube_shadow_look_up_gathers_a_texel_per_tap_and_level() {
    let rawdata = enum_value("sbmode", "rawdata");
    let replace = enum_value("lodm", "replace");
    let cases: &[(&str, bool, bool)] = &[
        /* expression, vertex, derivatives */
        ("texture(s, c)", false, true),
        ("texture(s, c, l)", false, true),
        ("textureGrad(s, c, g.xyz, h.xyz)", false, false),
        ("texture(s, c)", true, false),
        ("textureGrad(s, c, g.xyz, h.xyz)", true, false),
    ];
    for native in [true, false] {
        let dmn = enum_value("dmn", if native { "3d" } else { "2d" });
        for (e, vs, derivatives) in cases {
            let src = if *vs { vertex(&format!("v = vec4({e});")) } else { fragment(&format!("o = vec4({e});")) };
            let stage = if *vs { Stage::Vertex } else { Stage::Fragment };
            let (names, smps) =
                with_native_cube(native, || compile_with(stage, &src, ops).unwrap_or_else(|m| panic!("{e}: {m}")));
            assert_eq!(smps.len(), 9, "{e} (native {native}, vertex {vs}): the plain sample and eight gathers");
            assert_eq!((smps[0]["sbmode"], smps[0]["dmn"]), (0, dmn), "{e}: the plain sample first");
            for gather in &smps[1..] {
                assert_eq!(
                    (gather["sbmode"], gather["chan"], gather["lodm"], gather["pplod"], gather["tao"], gather["dmn"]),
                    (rawdata, 1, replace, 1, 0, dmn),
                    "{e} (native {native}, vertex {vs}): a gather at a replaced level"
                );
            }
            let quad = names.iter().filter(|n| matches!(**n, "fdsx" | "fdsy")).count();
            assert_eq!(quad > 0, *derivatives, "{e} (native {native}, vertex {vs}): {quad} derivatives");
        }
    }
}

fn sampler_marker(name: &str) -> u32 {
    usc_exec::named(7, name, 99)
}

/// A compiled fragment shader with the one sampler `s`: its code, and its
/// uniforms by name with their block dword and size.
struct Program {
    image: Vec<u8>,
    uniforms: Vec<(String, u16, u32)>,
}

fn compile_program(src: &str, native: bool) -> Program {
    with_native_cube(native, || {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("refused: {e:?}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let name_of = |n: &[core::ffi::c_char]| -> String {
                n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
            };
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            let samplers: Vec<String> =
                rows.iter().filter(|u| u.dword_off == no_block).map(|u| name_of(&u.name)).collect();
            assert_eq!(samplers, ["s"], "one sampler");
            Program {
                image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
                uniforms: rows
                    .iter()
                    .filter(|u| u.dword_off != no_block)
                    .map(|u| (name_of(&u.name), u.dword_off, u.dwords as u32))
                    .collect(),
            }
        })
        .unwrap()
    })
}

/// The program run with its descriptor's image words the model's, its
/// sampler meta words `meta` (dwords 12 and 13) and the uniforms `val`; the
/// packed pixel and the level words of the samples with a replaced level,
/// in program order.
fn run(p: &Program, meta: [u32; 2], val: &dyn Fn(&str, u32) -> u32) -> (u32, Vec<u32>) {
    usc_exec::set_smp_meta(meta);
    let marker = sampler_marker("s");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.shared[0..4].copy_from_slice(&usc_exec::tex_state(marker));
    inp.shared[4..7].copy_from_slice(&usc_exec::tex_meta(marker));
    inp.shared[12..14].copy_from_slice(&meta);
    for (name, off, dwords) in &p.uniforms {
        for k in 0..*dwords {
            inp.shared[20 + *off as usize + k as usize] = val(name, k);
        }
    }
    let _ = usc_exec::take_levels();
    let px = *usc_exec::run_binary(&p.image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
    (px, usc_exec::take_levels())
}

/// The filter words the byte-model runs take: every filter pair over a
/// chain of `levels` that compares per texel.
fn words(levels: u32) -> Vec<u32> {
    let mut w: Vec<u32> = MIN_FILTERS
        .iter()
        .flat_map(|min| [Filter::Nearest, Filter::Linear].map(move |mag| (*min, mag)))
        .map(|(min, mag)| filter_word(&Sampler { mag, min, wrap_s: Wrap::Repeat, wrap_t: Wrap::Repeat }, levels))
        .filter(|w| *w & 1 != 0)
        .collect();
    w.sort_unstable();
    w.dedup();
    w
}

/// The compiled programs compute `cube_model` with the byte model's texels:
/// on the native cube (each gather at its point's direction, which the
/// model maps through `cube_uv` as it maps every cube sample) and on the
/// stacked image (at (u, (face + v) / 6)), under `textureGrad`, a bias and
/// the implicit level (the byte model's derivatives, `fdsx` and `fdsy`, are
/// fixed functions of their operand), for every filter word over a chain
/// of five levels, at directions inside a face, by an edge and by a corner:
/// the eight gathers read the levels the model picks, and the pixel is the
/// model's result.
#[test]
fn the_program_computes_the_model_for_every_filter() {
    let marker = sampler_marker("s");
    let state = usc_exec::tex_state(marker);
    let size = |d: u32| usc_exec::tex_size_2d(state, d)[0];
    let n0 = size(0);
    /* a point by the edge at every level of the face */
    let edge = 1.0 - 0.4 / n0 as f32;
    let dirs: Vec<[f32; 3]> =
        [(0.0, 0.31, -0.42), (5.0, edge, 0.13), (2.0, -0.2, -edge), (3.0, edge, edge), (1.0, -edge, edge)]
            .iter()
            .map(|&(f, sc, tc)| face_direction(f, sc, tc).map(|c| c * 1.3))
            .collect();
    #[derive(Clone, Copy, Debug)]
    enum Lod {
        Grad,
        Bias,
        Implicit,
    }
    let forms: &[(&str, Lod)] = &[
        ("textureGrad(s, c, g.xyz, h.xyz)", Lod::Grad),
        ("texture(s, c, l)", Lod::Bias),
        ("texture(s, c)", Lod::Implicit),
    ];
    let lambdas = [-0.6f32, 0.2, 0.7, 1.3, 1.8, 2.6, 3.3, 6.0];
    let quad = |d: [f32; 3]| {
        let q = |op: &str| d.map(|x| f32::from_bits(usc_exec::alu(op, &[x.to_bits()]).unwrap()));
        [q("fdsx"), q("fdsy")]
    };
    let mut runs = 0;
    let mut seams = 0;
    for native in [true, false] {
        for (e, how) in forms {
            let program = compile_program(&fragment(&format!("o = vec4({e});")), native);
            for &dir in &dirs {
                let (face, st, _) = cube_face(dir, None);
                for (li, &target) in lambdas.iter().enumerate() {
                    /* the uniforms that make the level of detail `target`
                     * (for the implicit level, once: it is the quad's) */
                    if matches!(how, Lod::Implicit) && li > 0 {
                        continue;
                    }
                    let (g, h, lval, lambda) = match how {
                        Lod::Grad => {
                            let base = [[0.013f32, -0.007, 0.021], [-0.004, 0.017, 0.009]];
                            let (_, _, gr) = cube_face(dir, Some(base));
                            let k = (target - lambda_of(gr.unwrap(), n0, None)).exp2();
                            let dd = base.map(|v| v.map(|x| x * k));
                            let (_, _, gr) = cube_face(dir, Some(dd));
                            (dd[0], dd[1], 0.0, lambda_of(gr.unwrap(), n0, None))
                        }
                        Lod::Bias => {
                            let (_, _, gr) = cube_face(dir, Some(quad(dir)));
                            let l0 = lambda_of(gr.unwrap(), n0, None);
                            let b = target - l0;
                            ([0.0; 3], [0.0; 3], b, lambda_of(gr.unwrap(), n0, Some(b)))
                        }
                        Lod::Implicit => {
                            let (_, _, gr) = cube_face(dir, Some(quad(dir)));
                            ([0.0; 3], [0.0; 3], 0.0, lambda_of(gr.unwrap(), n0, None))
                        }
                    };
                    /* the texel a tap's gather reads in the byte model */
                    let texel = |d: u32, tap: &Tap| -> f32 {
                        let level = d as f32;
                        let (u, v) = if native {
                            let [sc, tc] = tap.at.map(|a| a * 2.0 + -1.0);
                            let p = face_direction(tap.face, sc, tc);
                            usc_exec::cube_uv(state, p.map(f32::to_bits), Some(level.to_bits()))
                        } else {
                            (tap.at[0].to_bits(), ((tap.at[1] + tap.face) * (1.0f32 / 6.0)).to_bits())
                        };
                        let m = usc_exec::marker_at_level(marker, level.to_bits());
                        f32::from_bits(usc_exec::gather_model(m, u, v, [0, 0])[0])
                    };
                    for word in words(5) {
                        let ([d1, d2], _, _) = pcf_levels(word, lambda);
                        let taps = level_taps(face, st, size(d1 as u32));
                        seams += usize::from(taps.iter().any(|t| t.face != face));
                        /* a reference between the first level's middle depths */
                        let mut sorted = taps.map(|t| texel(d1 as u32, &t));
                        sorted.sort_by(f32::total_cmp);
                        let dref = (sorted[1] + sorted[2]) * 0.5;
                        let lanes = [dir[0], dir[1], dir[2], dref];
                        let val = |n: &str, k: u32| -> u32 {
                            let k = k as usize;
                            match n {
                                "c" => lanes[k].to_bits(),
                                "g" if k < 3 => g[k].to_bits(),
                                "h" if k < 3 => h[k].to_bits(),
                                "l" => lval.to_bits(),
                                _ => 0,
                            }
                        };
                        for op in [1u32, 6] {
                            let (px, levels) = run(&program, [op, word], &val);
                            let n = levels.len();
                            assert!(n >= 8, "{e}: the gathers' levels: {levels:?}");
                            let got: Vec<f32> = levels[n - 8..].iter().map(|l| f32::from_bits(*l)).collect();
                            assert_eq!(
                                got,
                                [[d1; 4], [d2; 4]].concat(),
                                "{e} native {native} word {word:#x} lambda {lambda}: the gathers' levels"
                            );
                            let cmp = |x: f32| {
                                let holds = (op & 1 != 0 && dref < x)
                                    || (op & 2 != 0 && dref == x)
                                    || (op & 4 != 0 && x < dref);
                                if holds {
                                    1.0
                                } else {
                                    0.0
                                }
                            };
                            let want = cube_model(word, lambda, face, st, size, |d, t| cmp(texel(d, t)));
                            let want = (want.clamp(0.0, 1.0) * 255.0).round() as u32;
                            assert!(
                                (px & 0xff).abs_diff(want) <= 1,
                                "{e} native {native} dir {dir:?} word {word:#x} lambda {lambda} op {op}: \
                                 ours {}, the model {want}",
                                px & 0xff
                            );
                            runs += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(runs > 500, "{runs} runs");
    assert!(seams > 50, "{seams} footprints across an edge");
}
