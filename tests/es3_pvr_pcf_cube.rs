// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The compare per texel (PCF) of a `samplerCubeShadow` look-up on the
//! PowerVR online back end (select.rs `pcf_cube_taps`, on `pcf_levels` and
//! `shadow_compare` of the 2D PCF, tests/es3_pvr_pcf.rs): at the level(s)
//! the sampler's filter word (sampler meta dword 13) picks, the four texels
//! around `s * n - 0.5` on the face the direction selects are read one by
//! one, each compared, then weighted by `fract(s * n - 0.5)` and the two
//! levels mixed. ES 3.0 filters a cube seamlessly (3.8.9.1), and dEQP's
//! reference does (tcuTexture.cpp sampleCubeSeamlessLinearCompare): a tap
//! past a face edge reads the texel of the neighbouring face its centre
//! points at, a tap past a corner anything (dEQP does not check such a
//! footprint).
//!
//! - The arithmetic, on the CPU (`pcf_cube_model`, operation for operation
//!   as the program computes it), against the shared texture reference
//!   (src/shaderc/online/texref.rs `sample_cube_compare`, dEQP's seamless
//!   percentage-closer filter) over the six minification filters, both
//!   magnification filters, every face and its edges, outside the corners.
//! - The program's shape: the plain sample, then eight texel reads at a
//!   replaced level (two levels of four taps), no gather: on the native cube
//!   (`pvr_native_cube` on) samples of a direction (`dmn` 3d), on the
//!   stacked image texel fetches (`nncoords`); the quad's derivatives only
//!   where the level is implicit in a fragment shader.
//! - The compiled programs, in the byte model against `pcf_cube_model`
//!   with the model's own texture: the levels the taps read and the result,
//!   for every filter word, under the implicit level, a bias and
//!   `textureGrad`, on the native cube and the stacked image.
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

/* ---- the filter word and the levels (as tests/es3_pvr_pcf.rs) ------------ */

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

/// The level(s) `pcf_levels` reads at level of detail `lambda` under filter
/// word `word`, the blend weight of the second and whether the filter
/// within the level is NEAREST, each operation as the program computes it.
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

/// A direction's face as select.rs `cube_face` selects it (glsl2usc's
/// pvr_cubeUV): x major when |y| <= |x| and |z| <= |x|, else y major when
/// |z| <= |y|; the face coordinates `(sc / ma) * 0.5 + 0.5`.
#[derive(Clone, Copy, Debug)]
struct Face {
    face: f32,
    uv: [f32; 2],
    x_major: bool,
    y_major: bool,
    pos: [bool; 3],
}

fn cube_face([x, y, z]: [f32; 3]) -> Face {
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let x_major = ay <= ax && az <= ax;
    let y_major = az <= ay;
    let pos = [0.0 < x, 0.0 < y, 0.0 < z];
    let choose = |a: f32, b: f32, c: f32| {
        if x_major {
            a
        } else if y_major {
            b
        } else {
            c
        }
    };
    let ma = choose(ax, ay, az);
    let face = choose(if pos[0] { 0.0 } else { 1.0 }, if pos[1] { 2.0 } else { 3.0 }, if pos[2] { 4.0 } else { 5.0 });
    let u = choose(if pos[0] { -z } else { z }, x, if pos[2] { x } else { -x });
    let v = choose(-y, if pos[1] { z } else { -z }, -y);
    let r = 1.0 / ma;
    Face { face, uv: [u * r * 0.5 + 0.5, v * r * 0.5 + 0.5], x_major, y_major, pos }
}

/// select.rs `cube_face_point`: the point of `sel`'s face plane at face
/// coordinates (sc, tc) in [-1, 1].
fn face_point(sel: &Face, sc: f32, tc: f32) -> [f32; 3] {
    let choose = |a: f32, b: f32, c: f32| {
        if sel.x_major {
            a
        } else if sel.y_major {
            b
        } else {
            c
        }
    };
    let m = sel.pos.map(|p| if p { 1.0 } else { -1.0 });
    let x = choose(m[0], sc, if sel.pos[2] { sc } else { -sc });
    let y = choose(-tc, m[1], -tc);
    let z = choose(if sel.pos[0] { -sc } else { sc }, if sel.pos[1] { tc } else { -tc }, m[2]);
    [x, y, z]
}

/// One tap of a level's footprint as the program reads it: the face the
/// tap's centre selects, the texel (i, j) on it, and the direction through
/// that texel's centre (what the native cube samples); `corner` when the
/// tap lies past two edges of the look-up's face.
#[derive(Clone, Copy, Debug)]
struct Tap {
    face: f32,
    ij: [f32; 2],
    dir: [f32; 3],
    corner: bool,
}

/// The four taps (i0, j0), (i1, j0), (i0, j1), (i1, j1) on a face of `n`
/// texels, as `pcf_cube_taps` computes them.
fn footprint(sel: &Face, n: u32) -> [Tap; 4] {
    let nf = n as i32 as f32;
    let two_rn = (1.0 / nf) * 2.0;
    let last = nf + -1.0;
    let first = sel.uv.map(|x| (x * nf + -0.5).floor());
    let centre = |i: f32, step: f32| (i + (step + 0.5)) * two_rn + -1.0;
    [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(di, dj)| {
        let p = face_point(sel, centre(first[0], di), centre(first[1], dj));
        let on = cube_face(p);
        let ij = on.uv.map(|x| (x * nf).floor().max(0.0).min(last));
        let dir = face_point(&on, centre(ij[0], 0.0), centre(ij[1], 0.0));
        let out = |i: f32| i < 0.0 || i > last;
        Tap { face: on.face, ij, dir, corner: out(first[0] + di) && out(first[1] + dj) }
    })
}

/// What the program's PCF arm returns for a look-up at `dir`: the level(s)
/// from `lambda`, at each the four taps compared (`cmp`) and weighted at the
/// face size `size(level)`, the two levels mixed. `cmp(level, tap)` is the
/// compare result of the texel the tap reads.
fn pcf_cube_model(
    word: u32,
    lambda: f32,
    dir: [f32; 3],
    size: impl Fn(u32) -> u32,
    cmp: impl Fn(u32, &Tap) -> f32,
) -> f32 {
    let ([d1, d2], t, nearest) = pcf_levels(word, lambda);
    let sel = cube_face(dir);
    let at = |d: f32| {
        let n = size(d as u32);
        let c = footprint(&sel, n).map(|tap| cmp(d as u32, &tap));
        let weight = |x: f32| {
            let s = x * (n as i32 as f32) + -0.5;
            let fract = s + -s.floor();
            match (nearest, 0.5 <= fract) {
                (false, _) => fract,
                (true, true) => 1.0,
                (true, false) => 0.0,
            }
        };
        let (fu, fv) = (weight(sel.uv[0]), weight(sel.uv[1]));
        mix(mix(c[0], c[1], fu), mix(c[2], c[3], fu), fv)
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

/// An 8x8 depth cube and its full chain of four levels, R the depth.
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

/// Directions over every face, near its centre and its edges (but not on a
/// tie of two components, where the face is a tie rule's).
fn directions() -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    let span = [-0.97f32, -0.9, -0.71, -0.43, -0.12, 0.05, 0.31, 0.62, 0.88, 0.95];
    for axis in 0..3 {
        for sign in [1.0f32, -1.0] {
            for a in span {
                for b in span {
                    let mut d = [0.0; 3];
                    d[axis] = sign;
                    d[(axis + 1) % 3] = a;
                    d[(axis + 2) % 3] = b * 1.07;
                    out.push(d);
                }
            }
        }
    }
    out
}

/// The program's PCF arm, on the CPU, gives texref's seamless
/// percentage-closer result (each texel compared, then weighted; a tap past
/// an edge read on the neighbouring face) for every filter pair, level of
/// detail, face and reference texref predicts, outside the footprints with
/// a corner tap (texref takes the mean of the other three there, a
/// convention dEQP does not check); and the words that ask for no per texel
/// compare are exactly the filters whose one texel is the result.
#[test]
fn the_cube_pcf_arm_is_texrefs_seamless_percentage_closer_filter() {
    let t = cube_chain();
    let levels = t.levels.len() as u32;
    let lods = [-1.0f32, -0.25, 0.0, 0.3, 0.75, 1.0, 1.25, 1.6, 2.2, 2.75, 3.4, 5.0];
    let funcs = [CompareFunc::Less, CompareFunc::Gequal, CompareFunc::Lequal, CompareFunc::Greater];
    let size = |d: u32| t.levels[d as usize].width;
    let (mut checked, mut fractions, mut seams, mut corners) = (0usize, 0usize, 0usize, 0usize);
    for min in MIN_FILTERS {
        for mag in [Filter::Nearest, Filter::Linear] {
            let sp = Sampler { mag, min, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };
            let word = filter_word(&sp, levels);
            if word & 1 == 0 {
                /* the filtered depth's compare: NEAREST at the nearest
                 * level, the one texel the TPU's own sample reads */
                assert!(
                    mag == Filter::Nearest && sp.min.within_level() == Filter::Nearest && sp.min.mipmap() != Some(true)
                );
                continue;
            }
            for lod in lods {
                let ([d1, d2], _, nearest) = pcf_levels(word, lod);
                for (k, dir) in directions().into_iter().enumerate() {
                    let sel = cube_face(dir);
                    let taps: Vec<Tap> = [d1, d2].iter().flat_map(|d| footprint(&sel, size(*d as u32))).collect();
                    if !nearest && taps.iter().any(|tp| tp.corner) {
                        corners += 1;
                        continue;
                    }
                    let seam = taps.iter().any(|tp| tp.face != sel.face);
                    let dref = depth(3, k as u32, 0, 0, 0);
                    for func in funcs {
                        let cmp = Compare { func, dref, clamp_ref: false };
                        let Ok((want, face)) = texref::sample_cube_compare(
                            &t,
                            &sp,
                            &cmp,
                            dir[0],
                            dir[1],
                            dir[2],
                            texref::Lod::Explicit(lod),
                        ) else {
                            continue;
                        };
                        assert_eq!(face.index() as f32, sel.face, "{dir:?}: the face");
                        let texel = |d: u32, tp: &Tap| {
                            let l = &t.levels[d as usize];
                            let [i, j] = tp.ij.map(|x| x as u32);
                            cmp.texel(l.faces[tp.face as usize][(j * l.width + i) as usize])
                        };
                        let got = pcf_cube_model(word, lod, dir, size, texel);
                        assert!(
                            (got - want).abs() < 1e-4,
                            "{sp:?} word {word:#x}, lod {lod}, {dir:?}, {func:?} {dref}: the program {got}, texref {want}"
                        );
                        checked += 1;
                        fractions += usize::from(want != 0.0 && want != 1.0);
                        seams += usize::from(seam && !nearest);
                    }
                }
            }
        }
    }
    assert!(checked > 20_000, "texref predicted {checked} look-ups");
    assert!(fractions > 5_000, "{fractions} of them are fractions: the weights are exercised");
    assert!(seams > 2_000, "{seams} of them blend across a face edge");
    assert!(corners > 0, "{corners} footprints with a corner tap were left out");
}

/// A tap past an edge is the neighbouring face's texel along the shared
/// edge: on an 8x8 face, every tap of a footprint that leaves the face is
/// texref's `seamless_texel` of the same integer coordinates.
#[test]
fn a_tap_past_an_edge_is_texrefs_seamless_texel() {
    let t = cube_chain();
    let l = &t.levels[0];
    let mut past = 0;
    for dir in directions() {
        let sel = cube_face(dir);
        let first = sel.uv.map(|x| (x * 8.0 + -0.5).floor() as i32);
        for (tap, (di, dj)) in footprint(&sel, 8).into_iter().zip([(0, 0), (1, 0), (0, 1), (1, 1)]) {
            if tap.corner {
                continue;
            }
            let face = texref::Face::ALL[sel.face as usize];
            let want = texref::seamless_texel(l, face, first[0] + di, first[1] + dj).expect("not a corner");
            let [i, j] = tap.ij.map(|x| x as u32);
            assert_eq!(l.faces[tap.face as usize][(j * 8 + i) as usize], want, "{dir:?} tap ({di}, {dj})");
            past += usize::from(tap.face != sel.face);
        }
    }
    assert!(past > 100, "{past} taps crossed an edge");
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

/// Every instruction of a program, by name, with the `smp` fields.
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
                        ["sbmode", "chan", "lodm", "pplod", "dmn", "nncoords"]
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

/// Every form compiles on the native cube and on the stacked image, and the
/// program is the plain sample and eight texel reads at a replaced level
/// (no gather): directions (`dmn` 3d) on the native cube, texel fetches
/// (`nncoords`, `dmn` 2d) on the stacked image; the quad's derivatives
/// (`fdsx`, `fdsy`) only for a fragment shader's implicit level or bias.
#[test]
fn a_cube_shadow_look_up_reads_eight_texels() {
    let replace = enum_value("lodm", "replace");
    let (d3, d2) = (enum_value("dmn", "3d"), enum_value("dmn", "2d"));
    /* expression, vertex, derivatives */
    let cases: &[(&str, bool, bool)] = &[
        ("texture(s, c)", false, true),
        ("texture(s, c, l)", false, true),
        ("textureGrad(s, c, g.xyz, h.xyz)", false, false),
        ("texture(s, c)", true, false),
    ];
    for native in [true, false] {
        for (e, vs, derivatives) in cases {
            let (src, stage) = if *vs {
                (vertex(&format!("v = vec4({e});")), Stage::Vertex)
            } else {
                (fragment(&format!("o = vec4({e});")), Stage::Fragment)
            };
            let (names, smps) =
                with_native_cube(native, || compile_with(stage, &src, ops).unwrap_or_else(|m| panic!("{e}: {m}")));
            assert_eq!(smps.len(), 9, "{e} (native {native}, vertex {vs}): the plain sample and eight taps");
            for tap in &smps[1..] {
                let want = if native { (0, 4, replace, 1, d3, 0) } else { (0, 4, replace, 1, d2, 1) };
                assert_eq!(
                    (tap["sbmode"], tap["chan"], tap["lodm"], tap["pplod"], tap["dmn"], tap["nncoords"]),
                    want,
                    "{e} (native {native}, vertex {vs}): a texel at a replaced level"
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

/// Our program for a fragment shader with the one sampler `s`, its
/// descriptor's image words `state` (word 0 the model's marker) and its
/// sampler meta words `meta`
/// (dwords 12 and 13), the uniforms `val`; the packed pixel and the level
/// words of the samples with a replaced level, in program order.
fn run(src: &str, native: bool, state: [u32; 4], meta: [u32; 2], val: &dyn Fn(&str, u32) -> u32) -> (u32, Vec<u32>) {
    with_native_cube(native, || {
        usc_exec::set_smp_meta(meta);
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("refused: {e:?}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let name_of = |n: &[core::ffi::c_char]| -> String {
                n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
            };
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            let samplers: Vec<&yakogl::shaderc::ShcUniform> = rows.iter().filter(|u| u.dword_off == no_block).collect();
            assert_eq!(samplers.len(), 1, "one sampler");
            let marker = sampler_marker(&name_of(&samplers[0].name));
            assert_eq!(state[0], marker, "the image word the model samples");
            inp.shared[0..4].copy_from_slice(&state);
            inp.shared[4..7].copy_from_slice(&usc_exec::tex_meta(marker));
            inp.shared[12..14].copy_from_slice(&meta);
            for u in rows.iter().filter(|u| u.dword_off != no_block) {
                for k in 0..u.dwords as u32 {
                    inp.shared[20 + u.dword_off as usize + k as usize] = val(&name_of(&u.name), k);
                }
            }
            let _ = usc_exec::take_levels();
            let px = *usc_exec::run_binary(&image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
            (px, usc_exec::take_levels())
        })
        .unwrap()
    })
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

/// The level of detail the program computes for a look-up at `dir`: the
/// face coordinates' gradients -- `textureGrad`'s carried through the
/// projection (the quotient rule, as select.rs `cube_face` does), or the
/// byte model's quad derivatives of them -- at level 0's width and height
/// (a sixth of the stacked image's), as `pcf_levels` does.
fn model_lambda(state: [u32; 4], native: bool, dir: [f32; 3], grad: Option<[[f32; 3]; 2]>, bias: Option<f32>) -> f32 {
    let sel = cube_face(dir);
    let [w0, h0] = usc_exec::tex_size_2d(state, 0);
    let grads: [[f32; 2]; 2] = match grad {
        Some(dd) => {
            let [x, y, z] = dir;
            let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
            let r = 1.0
                / if sel.x_major {
                    ax
                } else if sel.y_major {
                    ay
                } else {
                    az
                };
            /* sc and tc as `cube_face` picks them (ES 3.0.6 table 3.21) */
            let sc = if sel.x_major {
                if sel.pos[0] {
                    -z
                } else {
                    z
                }
            } else if sel.y_major || sel.pos[2] {
                x
            } else {
                -x
            };
            let tc = if sel.x_major || !sel.y_major {
                -y
            } else if sel.pos[1] {
                z
            } else {
                -z
            };
            /* sc / ma, before the * 0.5 + 0.5 */
            let q = [sc * r, tc * r];
            let half_r = r * 0.5;
            dd.map(|[dx, dy, dz]| {
                let pk = |p: bool, a: f32| if p { a } else { -a };
                let dsc = if sel.x_major {
                    pk(!sel.pos[0], dz)
                } else if sel.y_major {
                    dx
                } else {
                    pk(sel.pos[2], dx)
                };
                let dtc = if sel.x_major || !sel.y_major { -dy } else { pk(sel.pos[1], dz) };
                let dm = if sel.x_major {
                    pk(sel.pos[0], dx)
                } else if sel.y_major {
                    pk(sel.pos[1], dy)
                } else {
                    pk(sel.pos[2], dz)
                };
                [(-q[0] * dm + dsc) * half_r, (-q[1] * dm + dtc) * half_r]
            })
        }
        None => {
            let d = |op: &str, x: f32| f32::from_bits(usc_exec::alu(op, &[x.to_bits()]).unwrap());
            [["fdsx"; 2], ["fdsy"; 2]].map(|[op, _]| [d(op, sel.uv[0]), d(op, sel.uv[1])])
        }
    };
    let wf = w0 as i32 as f32;
    let hf = if native { h0 as i32 as f32 } else { h0 as i32 as f32 * (1.0 / 6.0) };
    let len = |[du, dv]: [f32; 2]| {
        let (a, b) = (du * wf, dv * hf);
        b * b + a * a
    };
    let l2 = len(grads[0]).max(len(grads[1])).log2();
    match bias {
        Some(b) => l2 * 0.5 + b,
        None => l2 * 0.5,
    }
}

/// The compiled programs compute `pcf_cube_model` with the byte model's
/// texture: under the implicit level, a bias and `textureGrad`, on the
/// native cube and the stacked image, for every filter word over a chain of
/// five levels, at directions on four faces near and across their edges.
/// The eight taps read the levels the model picks, four each, and the pixel
/// is the model's result: on the native cube each tap is the sample of the
/// direction through its texel's centre (the model maps it to the stacked
/// image as select.rs `cube_uv` does), on the stacked image the fetch of
/// texel (i, face * n + j). The levels of detail sit away from the level
/// boundaries, where the program's `flog` and the model's log2 could round
/// to different sides.
#[test]
fn the_program_computes_the_model_for_every_filter() {
    let marker = sampler_marker("s");
    /* image words with a 64-texel face: 64 tall on the native cube, six
     * faces (384) on the stacked image; base level 0. Word 0 is the
     * marker the model samples, and the model's own native-cube mapping
     * (`usc_exec::cube_uv`) reads the marker's state, not these. */
    let state = |native: bool| {
        let h: u32 = if native { 64 } else { 6 * 64 };
        [marker, (63 << 2) | ((h - 1) << 16), usc_exec::tex_state(marker)[2], 0]
    };
    let face_size = |native: bool, d: u32| usc_exec::tex_size_2d(state(native), d)[0];
    #[derive(Clone, Copy, Debug)]
    enum Lod {
        Implicit,
        Bias,
        Grad,
    }
    let forms: &[(&str, Lod)] = &[
        ("texture(s, c)", Lod::Implicit),
        ("texture(s, c, l)", Lod::Bias),
        ("textureGrad(s, c, g.xyz, h.xyz)", Lod::Grad),
    ];
    let dirs = [[0.93f32, 0.41, -0.27], [-0.31, -0.97, 0.88], [0.62, -0.18, 0.999], [-0.55, 0.995, 0.2]];
    let biases = [-3.0f32, -1.2, 0.4, 1.7, 2.6];
    let (mut runs, mut boundary, mut fractions) = (0, 0, 0);
    for native in [true, false] {
        for (e, how) in forms {
            let src = fragment(&format!("o = vec4({e});"));
            for dir in dirs {
                for (bi, bias) in biases.into_iter().enumerate() {
                    /* the implicit level does not depend on the bias */
                    if matches!(how, Lod::Implicit) && bi > 0 {
                        continue;
                    }
                    /* the gradients: a length that makes a spread of levels */
                    let s = bias.exp2() / 32.0;
                    let grad = [[s, s * 0.3, -s * 0.2], [-s * 0.1, s * 0.7, s * 0.4]];
                    let lambda = match how {
                        Lod::Implicit => model_lambda(state(native), native, dir, None, None),
                        Lod::Bias => model_lambda(state(native), native, dir, None, Some(bias)),
                        Lod::Grad => model_lambda(state(native), native, dir, Some(grad), None),
                    };
                    let fr = lambda - lambda.floor();
                    if !(0.02..0.98).contains(&fr) || (fr - 0.5).abs() < 0.02 {
                        boundary += 1;
                        continue;
                    }
                    for word in words(5) {
                        let ([d1, d2], _, _) = pcf_levels(word, lambda);
                        /* the depth a tap reads in the byte model */
                        let tap_depth = |d: u32, tp: &Tap| -> f32 {
                            let level = (d as f32).to_bits();
                            let m = usc_exec::marker_at_level(marker, level);
                            let (u, v) = if native {
                                usc_exec::cube_uv(usc_exec::tex_state(marker), tp.dir.map(f32::to_bits), Some(level))
                            } else {
                                let n = face_size(native, d) as i32 as f32;
                                let col = usc_exec::texel_coord(tp.ij[0] + 0.5).unwrap();
                                let row = usc_exec::texel_coord(tp.face * n + (tp.ij[1] + 0.5)).unwrap();
                                (col, row)
                            };
                            f32::from_bits(usc_exec::sample_model(m, u, v, 0))
                        };
                        let sel = cube_face(dir);
                        let mut sorted =
                            footprint(&sel, face_size(native, d1 as u32)).map(|tp| tap_depth(d1 as u32, &tp));
                        sorted.sort_by(f32::total_cmp);
                        let dref = (sorted[1] + sorted[2]) * 0.5;
                        let val = |n: &str, k: u32| -> u32 {
                            match n {
                                "c" => [dir[0], dir[1], dir[2], dref][k as usize].to_bits(),
                                "g" => [grad[0][0], grad[0][1], grad[0][2], 0.0][k as usize].to_bits(),
                                "h" => [grad[1][0], grad[1][1], grad[1][2], 0.0][k as usize].to_bits(),
                                "l" => bias.to_bits(),
                                _ => 0,
                            }
                        };
                        for op in [1u32, 6] {
                            let (px, levels) = run(&src, native, state(native), [op, word], &val);
                            let n = levels.len();
                            assert!(n >= 8, "{e}: the taps' levels: {levels:?}");
                            let got: Vec<f32> = levels[n - 8..].iter().map(|b| f32::from_bits(*b)).collect();
                            assert_eq!(
                                got,
                                [d1, d1, d1, d1, d2, d2, d2, d2],
                                "{e} native {native} {dir:?} word {word:#x} lambda {lambda}: the taps' levels"
                            );
                            let cmp = |d: u32, tp: &Tap| {
                                let x = tap_depth(d, tp);
                                let holds = (op & 1 != 0 && dref < x)
                                    || (op & 2 != 0 && dref == x)
                                    || (op & 4 != 0 && x < dref);
                                if holds {
                                    1.0
                                } else {
                                    0.0
                                }
                            };
                            let want = pcf_cube_model(word, lambda, dir, |d| face_size(native, d), cmp);
                            let want = (want.clamp(0.0, 1.0) * 255.0).round() as u32;
                            assert!(
                                (px & 0xff).abs_diff(want) <= 1,
                                "{e} native {native} {dir:?} word {word:#x} lambda {lambda} op {op}: ours {}, the model {want}",
                                px & 0xff
                            );
                            runs += 1;
                            fractions += usize::from(want != 0 && want != 255);
                        }
                    }
                }
            }
        }
    }
    assert!(runs > 300, "{runs} runs ({boundary} levels of detail left out at a boundary)");
    assert!(fractions > runs / 4, "{fractions} of {runs} results are fractions: the weights are exercised");
}
