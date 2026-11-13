// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The compare per texel (PCF) of a shadow look-up on the PowerVR online back
//! end, for every filter a sampler has (docs/es3/pvr-2darray.md section
//! 7.2; select.rs `pcf_taps` and `shadow_compare`, src/pvr/backend.rs
//! `shadow_filter_word`): the program computes the level of detail, takes
//! the level a `*_MIPMAP_NEAREST` filter reads or the two a
//! `*_MIPMAP_LINEAR` filter blends, gathers the bilinear footprint at each
//! (pco's `tg4`), compares each tap, weights the four results by
//! `fract(coord * size - 0.5)` at that level's size (rounded to 0 or 1 for a
//! NEAREST filter within the level) and mixes the two levels by the
//! fraction of the level of detail. The filter is sampler meta dword 13, so
//! one program serves every filter and wrap mode.
//!
//! - The program's shape: the plain sample and two gathers at a replaced
//!   level, on a 2D sampler, a native 2D array (`tao`) and a stacked one,
//!   the quad's derivatives only where the level is implicit in a fragment
//!   shader.
//! - The arithmetic, on the CPU (`pcf_model`, operation for operation as
//!   the program computes it), against the shared texture reference
//!   (src/shaderc/online/texref.rs `sample_layer_compare`, dEQP's
//!   percentage-closer filter) over the six minification filters, both
//!   magnification filters and the three wrap modes: MIRRORED_REPEAT
//!   included, whose weights are those of the unwrapped coordinate because
//!   the TPU's gather wraps each tap index (pco's tg4 changes neither the
//!   coordinate nor the tap order).
//! - The compiled programs, in the byte model against `pcf_model`: the
//!   levels the gathers read and the result, for each filter word, under
//!   `textureLod`, `textureGrad`, a bias and the implicit level.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_pcf
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

/// The native-array switch set for the closure, and back at its default
/// after it, panic or not.
fn with_native_array<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_array(i32::from(on));
    f()
}

/* ---- the filter word ----------------------------------------------------- */

/// Sampler meta dword 13 as src/pvr/backend.rs `shadow_filter_word` writes
/// it for `sp` over `levels` levels (the back end's own unit test pins the
/// function; this is its specification, written over texref's sampler).
fn filter_word(sp: &Sampler, levels: u32) -> u32 {
    let mag_linear = sp.mag == Filter::Linear;
    let min_linear = sp.min.within_level() == Filter::Linear;
    let chain = levels > 1;
    let mip = if chain { sp.min.mipmap() } else { None };
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

/* ---- the program's arithmetic on the CPU ---------------------------------- */

/// The level(s) `pcf_taps` reads at level of detail `lambda` under filter
/// word `word`, and the blend weight of the second and whether the filter
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
    /* the program's ffma(lambda, -1, -0.5), exact */
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

/// One level's weighted compares: `c` the four taps' compares in gather
/// order, `(u, v)` the coordinate, `(w, h)` the level's size.
fn level_pcf(c: [f32; 4], [u, v]: [f32; 2], [w, h]: [u32; 2], nearest: bool) -> f32 {
    let weight = |x: f32, n: u32| {
        let s = x * (n as i32 as f32) + -0.5;
        let fract = s + -s.floor();
        match (nearest, 0.5 <= fract) {
            (false, _) => fract,
            (true, true) => 1.0,
            (true, false) => 0.0,
        }
    };
    let (fu, fv) = (weight(u, w), weight(v, h));
    mix(mix(c[0], c[1], fu), mix(c[2], c[3], fu), fv)
}

/// What the program's PCF arm returns: the level(s) from `lambda`, at each
/// the compares of the taps `gather(level)` returns weighted at the size
/// `size(level)`, the two mixed.
fn pcf_model(
    word: u32,
    lambda: f32,
    uv: [f32; 2],
    gather: impl Fn(u32) -> [f32; 4],
    size: impl Fn(u32) -> [u32; 2],
) -> f32 {
    let ([d1, d2], t, nearest) = pcf_levels(word, lambda);
    let at = |d: f32| level_pcf(gather(d as u32), uv, size(d as u32), nearest);
    mix(at(d1), at(d2), t)
}

/* ---- against texref ------------------------------------------------------ */

/// A deterministic depth in [0, 1) per texel.
fn depth(seed: u32, level: u32, layer: u32, i: u32, j: u32) -> f32 {
    let mut h = seed ^ 0x9e37_79b9;
    for x in [level, layer, i, j] {
        h = (h ^ x).wrapping_mul(0x0100_0193).rotate_left(13);
    }
    (h % 997) as f32 / 997.0
}

/// A 16x8 depth texture of `layers` layers and its full chain of five
/// levels, R the depth.
fn chain(layers: u32) -> Texture {
    let mut levels = Vec::new();
    let (mut w, mut h) = (16u32, 8u32);
    for l in 0..5 {
        let faces =
            (0..layers).map(|k| (0..w * h).map(|n| [depth(7, l, k, n % w, n / w), 0.0, 0.0, 1.0]).collect()).collect();
        levels.push(Level { width: w, height: h, faces });
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
    Texture { cube: false, min: MinFilter::Nearest, levels }
}

/// The TPU's gather at level `d` of `t`'s layer `layer` (pco's tg4: the
/// bilinear footprint, each tap index wrapped by the sampler's mode as a
/// LINEAR filter wraps it): R of (i0, j0), (i1, j0), (i0, j1), (i1, j1),
/// i0 = floor(s * w - 0.5).
fn tpu_gather(t: &Texture, sp: &Sampler, layer: usize, d: u32, [s, tc]: [f32; 2]) -> [[f32; 4]; 4] {
    let l = &t.levels[d as usize];
    let (x0, y0) = ((s * l.width as f32 - 0.5).floor() as i32, (tc * l.height as f32 - 0.5).floor() as i32);
    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| {
        let i = texref::wrap_index(sp.wrap_s, x0 + dx, l.width);
        let j = texref::wrap_index(sp.wrap_t, y0 + dy, l.height);
        l.faces[layer][(j * l.width + i) as usize]
    })
}

const MIN_FILTERS: [MinFilter; 6] = [
    MinFilter::Nearest,
    MinFilter::Linear,
    MinFilter::NearestMipmapNearest,
    MinFilter::LinearMipmapNearest,
    MinFilter::NearestMipmapLinear,
    MinFilter::LinearMipmapLinear,
];
const WRAPS: [Wrap; 3] = [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat];

/// The program's PCF arm, on the CPU with the TPU's gather, gives texref's
/// percentage-closer result (compare every texel the filter reads, then
/// weight; dEQP's sampleLevelArray2DCompare) for every filter pair, wrap
/// mode, level of detail, reference and layer texref predicts -- 2D (one
/// layer) and 2D array (three) alike; and the words that ask for no per
/// texel compare are exactly the filters whose one texel is the result.
#[test]
fn the_pcf_arm_is_texrefs_percentage_closer_filter() {
    let lods = [-1.0f32, -0.25, 0.0, 0.3, 0.75, 1.0, 1.25, 1.6, 2.2, 2.75, 3.4, 4.0, 4.7, 7.0];
    let funcs = [CompareFunc::Less, CompareFunc::Gequal, CompareFunc::Lequal, CompareFunc::Greater];
    let mut checked = 0usize;
    let mut fractions = 0usize;
    for layers in [1u32, 3] {
        let t = chain(layers);
        let levels = t.levels.len() as u32;
        for min in MIN_FILTERS {
            for mag in [Filter::Nearest, Filter::Linear] {
                for (ws, wt) in WRAPS.iter().flat_map(|a| WRAPS.iter().map(move |b| (*a, *b))) {
                    let sp = Sampler { mag, min, wrap_s: ws, wrap_t: wt };
                    let word = filter_word(&sp, levels);
                    for lod in lods {
                        for k in 0..23 {
                            /* coordinates over [-1.4, 2.4], into the
                             * neighbouring wrap periods both ways */
                            let s = -1.4 + 3.8 * ((k * 7 % 23) as f32 + 0.37) / 23.0;
                            let tc = -1.4 + 3.8 * ((k * 11 % 23) as f32 + 0.61) / 23.0;
                            let layer = (k as usize) % layers as usize;
                            let dref = depth(3, k, 0, 0, 0);
                            for func in funcs {
                                let cmp = Compare { func, dref, clamp_ref: false };
                                let Ok(want) = texref::sample_layer_compare(
                                    &t,
                                    &sp,
                                    &cmp,
                                    layer,
                                    s,
                                    tc,
                                    texref::Lod::Explicit(lod),
                                ) else {
                                    continue;
                                };
                                let gather = |d: u32| tpu_gather(&t, &sp, layer, d, [s, tc]).map(|x| cmp.texel(x));
                                let size = |d: u32| [t.levels[d as usize].width, t.levels[d as usize].height];
                                let got = if word & 1 != 0 {
                                    pcf_model(word, lod, [s, tc], gather, size)
                                } else {
                                    /* the filtered depth's compare: the
                                     * TPU's one texel (NEAREST at the
                                     * nearest level) */
                                    let l = texref::levels(&t, sp.min, texref::Lod::Explicit(lod)).unwrap().0;
                                    let lv = &t.levels[l];
                                    cmp.texel(
                                        texref::sample_face(lv, layer, Filter::Nearest, &sp, s, tc)
                                            .expect("texref predicted the compare"),
                                    )
                                };
                                assert!(
                                    (got - want).abs() < 1e-4,
                                    "{sp:?} word {word:#x}, {layers} layer(s), layer {layer}, lod {lod}, \
                                     ({s}, {tc}), {func:?} {dref}: the program {got}, texref {want}"
                                );
                                checked += 1;
                                fractions += usize::from(want != 0.0 && want != 1.0);
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 50_000, "texref predicted {checked} look-ups");
    assert!(fractions > 10_000, "{fractions} of them are fractions: the weights are exercised");
}

/// MIRRORED_REPEAT needs no weights of its own: with the gather's taps in
/// index order, weights taken from the mirrored coordinate (1 - fract in a
/// mirrored period) would swap the two columns' weights and miss texref,
/// while the unwrapped coordinate's (what the program takes) meet it.
#[test]
fn mirrored_repeat_takes_the_unwrapped_coordinates_weights() {
    let t = chain(1);
    let sp =
        Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s: Wrap::MirroredRepeat, wrap_t: Wrap::Repeat };
    /* the depths themselves weighted, against texref's plain LINEAR blend:
     * the weights are the question, not the compare; s in the mirrored
     * period [1, 2) */
    let (s, tc) = (1.0 + 4.3 / 16.0, 2.7 / 8.0);
    let taps = tpu_gather(&t, &sp, 0, 0, [s, tc]).map(|x| x[0]);
    let want = texref::sample_layer(&t, &sp, 0, s, tc, texref::Lod::Explicit(0.0)).unwrap()[0];
    let unwrapped = level_pcf(taps, [s, tc], [16, 8], false);
    assert!((unwrapped - want).abs() < 1e-5, "unwrapped weights {unwrapped}, texref {want}");
    let mirrored_s = 1.0 - (s - s.floor());
    let mirrored = level_pcf(taps, [mirrored_s, tc], [16, 8], false);
    assert!((mirrored - want).abs() > 1e-3, "the mirrored coordinate's weights would give {mirrored}, texref {want}");
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
                        ["sbmode", "chan", "lodm", "pplod", "tao"]
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

const DECLS: &str = "uniform vec4 c;\nuniform vec4 g;\nuniform float l;\n";

fn fragment(sampler: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n{DECLS}\
         out vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(sampler: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n{DECLS}\
         in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; {body} }}\n"
    )
}

fn stacked(src: &str) -> String {
    src.replace("uniform highp", "uniform float pvr_layers_s;\nuniform highp")
}

/// The program's shape is the same for every filter and wrap mode (they are
/// the sampler's, read when the shader runs): the plain sample, then two
/// gathers -- `sbmode` rawdata, one channel, `lodm` replace -- carrying
/// `tao` on a native 2D array and not on a stacked one; the quad's
/// derivatives (`fdsx`, `fdsy`) only for a fragment shader's implicit level
/// or bias, none for `textureLod`, `textureGrad` or a vertex shader.
#[test]
fn a_shadow_look_up_gathers_at_two_levels() {
    let rawdata = enum_value("sbmode", "rawdata");
    let replace = enum_value("lodm", "replace");
    let cases: &[(&str, &str, bool, bool, bool)] = &[
        /* sampler, expression, native array, vertex, derivatives */
        ("sampler2DShadow", "texture(s, c.xyz)", false, false, true),
        ("sampler2DShadow", "texture(s, c.xyz, l)", false, false, true),
        ("sampler2DShadow", "textureProj(s, c)", false, false, true),
        ("sampler2DShadow", "textureLod(s, c.xyz, l)", false, false, false),
        ("sampler2DShadow", "textureGrad(s, c.xyz, g.xy, g.zw)", false, false, false),
        ("sampler2DShadow", "textureOffset(s, c.xyz, ivec2(3, -2))", false, false, true),
        ("sampler2DShadow", "texture(s, c.xyz)", false, true, false),
        ("sampler2DShadow", "textureLod(s, c.xyz, l)", false, true, false),
        ("sampler2DArrayShadow", "texture(s, c)", true, false, true),
        ("sampler2DArrayShadow", "textureGrad(s, c, g.xy, g.zw)", true, false, false),
        ("sampler2DArrayShadow", "texture(s, c)", false, false, true),
        ("sampler2DArrayShadow", "textureGradOffset(s, c, g.xy, g.zw, ivec2(1, -1))", false, false, false),
        ("sampler2DArrayShadow", "texture(s, c)", false, true, false),
    ];
    for (sampler, e, native, vs, derivatives) in cases {
        let src = if *vs {
            vertex(sampler, &format!("v = vec4({e});"))
        } else {
            fragment(sampler, &format!("o = vec4({e});"))
        };
        let src = if *sampler == "sampler2DArrayShadow" && !native { stacked(&src) } else { src };
        let stage = if *vs { Stage::Vertex } else { Stage::Fragment };
        let (names, smps) = with_native_array(*native, || {
            compile_with(stage, &src, ops).unwrap_or_else(|m| panic!("{sampler} {e}: {m}"))
        });
        assert_eq!(smps.len(), 3, "{sampler} {e} (native {native}, vertex {vs}): the plain sample and two gathers");
        let tao = u32::from(*native);
        assert_eq!(smps[0]["sbmode"], 0, "{e}: the plain sample first");
        for gather in &smps[1..] {
            assert_eq!(
                (gather["sbmode"], gather["chan"], gather["lodm"], gather["pplod"], gather["tao"]),
                (rawdata, 1, replace, 1, tao),
                "{sampler} {e} (native {native}, vertex {vs}): a gather at a replaced level"
            );
        }
        let quad = names.iter().filter(|n| matches!(**n, "fdsx" | "fdsy")).count();
        assert_eq!(quad > 0, *derivatives, "{sampler} {e} (native {native}, vertex {vs}): {quad} derivatives");
    }
    /* a cube gathers nothing: its taps are read one by one
     * (tests/es3_pvr_pcf_cube.rs) */
    let fs = fragment("samplerCubeShadow", "o = vec4(texture(s, c));");
    let (_, smps) = with_native_array(false, || compile_with(Stage::Fragment, &fs, ops).unwrap());
    assert!(smps.iter().all(|s| s["sbmode"] == 0), "a cube shadow look-up: no gather");
}

fn sampler_marker(name: &str) -> u32 {
    usc_exec::named(7, name, 99)
}

/// Our program for a fragment shader with the one sampler `s`, its
/// descriptor's image words the model's and its sampler meta words `meta`
/// (dwords 12 and 13), the uniforms `val`; the packed pixel and the level
/// words of the samples with a replaced level, in program order.
fn run(src: &str, native: bool, meta: [u32; 2], val: &dyn Fn(&str, u32) -> u32) -> (u32, Vec<u32>) {
    with_native_array(native, || {
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
            let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                rows.iter().filter(|u| u.dword_off == no_block).collect();
            samplers.sort_by_key(|u| u.binding);
            assert_eq!(samplers.len(), 1, "one sampler");
            let marker = sampler_marker(&name_of(&samplers[0].name));
            inp.shared[0..4].copy_from_slice(&usc_exec::tex_state(marker));
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
/// chain of `levels` (the back end's words for them), and a few more.
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

/// The compiled programs compute `pcf_model`: on a 2D sampler under
/// `textureLod`, `textureGrad`, a bias and the implicit level (the byte
/// model's derivatives, `fdsx` and `fdsy`, are fixed functions of their
/// operand), on a native 2D array and on a stacked one, for every filter
/// word over a chain of five levels: the two gathers read the levels the
/// model picks, and the pixel is the model's result. The level-of-detail
/// values sit away from the level boundaries, where the program's `flog`
/// and the model's log2 could round to different sides.
#[test]
fn the_program_computes_the_model_for_every_filter() {
    let marker = sampler_marker("s");
    let state = usc_exec::tex_state(marker);
    let size = |d: u32| usc_exec::tex_size_2d(state, d);
    let [w0, h0] = size(0);
    let (u, v) = (0.3137f32, 0.7712f32);
    /* (sampler, expression, native) and how the program's level of detail
     * follows from the uniforms */
    #[derive(Clone, Copy, Debug)]
    enum Lod {
        Explicit,
        Grad,
        Bias,
        Implicit,
    }
    let forms: &[(&str, &str, bool, Lod)] = &[
        ("sampler2DShadow", "textureLod(s, c.xyz, l)", false, Lod::Explicit),
        ("sampler2DShadow", "textureGrad(s, c.xyz, g.xy, g.zw)", false, Lod::Grad),
        ("sampler2DShadow", "texture(s, c.xyz, l)", false, Lod::Bias),
        ("sampler2DShadow", "texture(s, c.xyz)", false, Lod::Implicit),
        ("sampler2DArrayShadow", "textureGrad(s, c, g.xy, g.zw)", true, Lod::Grad),
        ("sampler2DArrayShadow", "textureGrad(s, c, g.xy, g.zw)", false, Lod::Grad),
    ];
    /* the gradients' length in texels, 2^lambda, for chosen lambdas */
    let lambdas = [-0.6f32, 0.2, 0.7, 1.3, 1.8, 2.6, 3.3, 6.0];
    let layers = 4.0f32;
    let mut runs = 0;
    for (sampler, e, native, how) in forms {
        let array = *sampler == "sampler2DArrayShadow";
        let src = fragment(sampler, &format!("o = vec4({e});"));
        let src = if array && !native { stacked(&src) } else { src };
        for word in words(5) {
            for lambda in lambdas {
                /* the uniforms that make the program's level of detail
                 * `lambda` (or, for the bias and the implicit level, the
                 * model's level of detail from them) */
                let gx = lambda.exp2() / w0 as f32;
                let (cvec, gvec, lval) = match how {
                    Lod::Explicit => ([u, v, 0.0, 0.0], [0.0; 4], lambda),
                    Lod::Grad => ([u, v, 0.0, 0.0], [gx, 0.0, 0.0, gx * 0.5 * w0 as f32 / h0 as f32], 0.0),
                    Lod::Bias => ([u, v, 0.0, 0.0], [0.0; 4], lambda),
                    Lod::Implicit => ([u, v, 0.0, 0.0], [0.0; 4], 0.0),
                };
                let model_lambda = match how {
                    Lod::Explicit => lambda,
                    Lod::Grad => {
                        /* a stacked array's layer is 1 / layers of the
                         * image's height */
                        let hf = if array && !native { h0 as i32 as f32 * (1.0 / layers) } else { h0 as i32 as f32 };
                        let len = |du: f32, dv: f32| {
                            let (a, b) = (du * w0 as i32 as f32, dv * hf);
                            b * b + a * a
                        };
                        len(gvec[0], gvec[1]).max(len(gvec[2], gvec[3])).log2() * 0.5
                    }
                    Lod::Bias | Lod::Implicit => {
                        let d = |op: &str, x: f32| f32::from_bits(usc_exec::alu(op, &[x.to_bits()]).unwrap());
                        let len = |op: &str| {
                            let (a, b) = (d(op, cvec[0]) * w0 as i32 as f32, d(op, cvec[1]) * h0 as i32 as f32);
                            b * b + a * a
                        };
                        let l2 = len("fdsx").max(len("fdsy")).log2();
                        if matches!(how, Lod::Bias) {
                            l2 * 0.5 + lval
                        } else {
                            l2 * 0.5
                        }
                    }
                };
                let ([d1, d2], _, _) = pcf_levels(word, model_lambda);
                /* where the texels sampled are, per form: the coordinate
                 * and the texture of a level */
                let layer = 2u32;
                let c_lanes = if array { [u, v, layer as f32, 0.0] } else { [u, v, 0.0, 0.0] };
                let uv = if array && !native {
                    let (su, sv) = usc_exec::array_uv(
                        state,
                        layers.to_bits(),
                        [u.to_bits(), v.to_bits(), (layer as f32).to_bits()],
                    );
                    [f32::from_bits(su), f32::from_bits(sv)]
                } else {
                    [u, v]
                };
                let texture = |d: f32| {
                    let m = usc_exec::marker_at_level(marker, d.to_bits());
                    if array && *native {
                        usc_exec::marker_at_layer(m, layer)
                    } else {
                        m
                    }
                };
                let taps = |d: f32| {
                    usc_exec::gather_model(texture(d), uv[0].to_bits(), uv[1].to_bits(), [0, 0]).map(f32::from_bits)
                };
                /* a reference between the first level's middle depths */
                let mut sorted = taps(d1);
                sorted.sort_by(f32::total_cmp);
                let dref = (sorted[1] + sorted[2]) * 0.5;
                let lanes = if array { [c_lanes[0], c_lanes[1], c_lanes[2], dref] } else { [u, v, dref, 1.0] };
                let val = |n: &str, k: u32| -> u32 {
                    match n {
                        "c" => lanes[k as usize].to_bits(),
                        "g" => gvec[k as usize].to_bits(),
                        "l" => lval.to_bits(),
                        "pvr_layers_s" => layers.to_bits(),
                        _ => 0,
                    }
                };
                for op in [1u32, 6] {
                    let (px, levels) = run(&src, *native, [op, word], &val);
                    let n = levels.len();
                    assert!(n >= 2, "{e}: the gathers' levels: {levels:?}");
                    assert_eq!(
                        [f32::from_bits(levels[n - 2]), f32::from_bits(levels[n - 1])],
                        [d1, d2],
                        "{sampler} {e} native {native} word {word:#x} lambda {model_lambda}: the gathers' levels"
                    );
                    let cmp = |x: f32| {
                        let holds =
                            (op & 1 != 0 && dref < x) || (op & 2 != 0 && dref == x) || (op & 4 != 0 && x < dref);
                        if holds {
                            1.0
                        } else {
                            0.0
                        }
                    };
                    let want = pcf_model(word, model_lambda, uv, |d| taps(d as f32).map(cmp), size);
                    let want = (want.clamp(0.0, 1.0) * 255.0).round() as u32;
                    assert!(
                        (px & 0xff).abs_diff(want) <= 1,
                        "{sampler} {e} native {native} word {word:#x} lambda {model_lambda} op {op}: \
                         ours {}, the model {want}",
                        px & 0xff
                    );
                    runs += 1;
                }
            }
        }
    }
    assert!(runs > 300, "{runs} runs");
}
