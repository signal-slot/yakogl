// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D samples the classes the six decoded ETC2/EAC formats are stored
//! in (docs/notes/etc2-eac.md sections 1 and 3): R16F (R11_EAC, signed and
//! unsigned), RGBA16F (RG11_EAC, signed and unsigned), RGBA8
//! (RGB8_PUNCHTHROUGH_ALPHA1_ETC2) and SRGB8_ALPHA8
//! (SRGB8_PUNCHTHROUGH_ALPHA1_ETC2), within dEQP's thresholds.
//!
//! What this is, and is not. The QPU interpreter (`v3d::sim`) takes a
//! texture as f32 texels (`texref::Texture`): it does not see GL, glTexImage2D
//! or the TMU's format decode. So the test does the GL core's store and the
//! TMU's read-back HERE, the way the hardware row would: each decoded value
//! is quantised by the class (`es3_formats::f32_to_half` for the half
//! classes, a byte for RGBA8 and SRGB8_ALPHA8, whose colour the TMU decodes
//! with `srgb::decode`), and that is the interpreter's texel. The compiled
//! `texture()` look-up then runs on the interpreter with the draw's sampler
//! (NEAREST or LINEAR, wrap), and its f16 result is compared with a CPU
//! reference computed from the UNQUANTISED decoded values (the number dEQP's
//! reference decoder holds). So the error reported is class quantisation +
//! interpreter filtering + the f16 the fragment writes. What it cannot show:
//! the TMU's real texture type and swizzle for the class (pinned by
//! tests/es3_formats_v3d.rs and tools/v3d/tex_check.c) and a real GL upload
//! (the host readback is black; the GL core's decode is src/gl/etc2.rs).
//!
//! Thresholds (etc2-eac.md section 3): NEAREST, unsigned 0.0078585
//! (7 bits), signed 0.015717, sRGB 0.015702, texture.format 2/255 (RGBA8);
//! LINEAR uses dEQP's 6-bit pair (0.015702, signed 0.031403).
//!
//! The largest error of every class and filter is printed (`--nocapture`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_decoded_class_v3d -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use std::sync::Mutex;
use yakogl::gl::es3_formats::{f32_to_half, half_to_f32};
use yakogl::gl::srgb;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{self, Face, Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program, Code, Prim, Target};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());
/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    R16F,
    Rgba16F,
    Rgba8,
    Srgb8A8,
}

fn byte(x: f32) -> u8 {
    (x * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// What the TMU returns for a texel the GL core stored in `class` from the
/// decoded value `v`.
fn stored(class: Class, v: [f32; 4]) -> [f32; 4] {
    let h = |x: f32| half_to_f32(f32_to_half(x));
    match class {
        Class::R16F => [h(v[0]), 0.0, 0.0, 1.0],
        Class::Rgba16F => v.map(h),
        Class::Rgba8 => v.map(|x| byte(x) as f32 / 255.0),
        Class::Srgb8A8 => {
            [srgb::decode(byte(v[0])), srgb::decode(byte(v[1])), srgb::decode(byte(v[2])), byte(v[3]) as f32 / 255.0]
        }
    }
}

/// The value dEQP's reference holds for a decoded colour: the exact linear
/// colour (an sRGB byte decoded) for the sRGB class, an R-only texel for R16F.
fn truth(class: Class, v: [f32; 4]) -> [f32; 4] {
    match class {
        Class::Srgb8A8 => [srgb::decode(byte(v[0])), srgb::decode(byte(v[1])), srgb::decode(byte(v[2])), v[3]],
        Class::R16F => [v[0], 0.0, 0.0, 1.0],
        _ => v,
    }
}

/// dEQP's `extend11To16` of an unsigned 11-bit EAC code, as UNORM_INT16.
fn eac_u(code: u32) -> f32 {
    (((code << 5) | (code >> 6)) as f32) / 65535.0
}

/// A signed 11-bit EAC code (-1023..=1023) as SNORM_INT16.
fn eac_s(code: i32) -> f32 {
    let m = code.unsigned_abs();
    let e = ((m << 5) | (m >> 5)) as i32;
    let e = if code < 0 { -e } else { e };
    (e as f32 / 32767.0).max(-1.0)
}

/// A deterministic sequence of 11-bit codes with the extremes first.
fn codes(n: usize, signed: bool) -> Vec<i32> {
    let edge: [i32; 8] =
        if signed { [0, 1, -1, 1023, -1023, 512, -512, 7] } else { [0, 1, 2047, 1024, 1023, 512, 1535, 7] };
    let mut x = 0x2545_f491u32;
    (0..n)
        .map(|i| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            if i < edge.len() {
                edge[i]
            } else if signed {
                ((x >> 8) % 2047) as i32 - 1023
            } else {
                ((x >> 8) % 2048) as i32
            }
        })
        .collect()
}

/// `n` decoded texels of `class`: EAC codes in the half classes, random
/// colours with alpha 0 or 1 (punch-through) in the byte classes.
fn texels_of(class: Class, signed: bool, n: usize) -> Vec<[f32; 4]> {
    let c = codes(n * 2, signed);
    let val = |k: i32| if signed { eac_s(k) } else { eac_u(k as u32) };
    match class {
        Class::R16F => (0..n).map(|i| [val(c[i]), 0.0, 0.0, 1.0]).collect(),
        Class::Rgba16F => (0..n).map(|i| [val(c[i]), val(c[n + i]), 0.0, 1.0]).collect(),
        Class::Rgba8 | Class::Srgb8A8 => {
            let mut x = 0x1234_5678u32;
            (0..n)
                .map(|i| {
                    let mut t = [0.0f32; 4];
                    for ch in &mut t[..3] {
                        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                        *ch = ((x >> 16) & 0xff) as f32 / 255.0;
                    }
                    t[3] = if i % 3 == 1 { 0.0 } else { 1.0 };
                    t
                })
                .collect()
        }
    }
}

fn tex_of(class: Class, w: u32, h: u32, orig: &[[f32; 4]], cube: bool, faces: usize) -> Texture {
    let n = (w * h) as usize;
    let faces = (0..faces).map(|f| orig[f * n..(f + 1) * n].iter().map(|&t| stored(class, t)).collect()).collect();
    Texture { cube, min: MinFilter::Nearest, levels: vec![Level { width: w, height: h, faces }] }
}

// ---- the reference: ES 3.0.6 3.8.9 --------------------------------------------

fn wrap(mode: Wrap, i: i32, n: i32) -> i32 {
    match mode {
        Wrap::ClampToEdge => i.clamp(0, n - 1),
        Wrap::Repeat => i.rem_euclid(n),
        Wrap::MirroredRepeat => {
            let p = i.rem_euclid(2 * n);
            if p < n {
                p
            } else {
                2 * n - 1 - p
            }
        }
    }
}

/// NEAREST: the texel the wrapped coordinate falls in; LINEAR: the four
/// texels around it, wrapped by index, blended.
fn reference(texels: &[[f32; 4]], w: i32, h: i32, sp: Sampler, s: f32, t: f32, class: Class) -> [f32; 4] {
    let at = |i: i32, j: i32| truth(class, texels[(wrap(sp.wrap_t, j, h) * w + wrap(sp.wrap_s, i, w)) as usize]);
    let (u, v) = (s * w as f32, t * h as f32);
    if sp.mag == Filter::Nearest {
        return at(u.floor() as i32, v.floor() as i32);
    }
    let (u, v) = (u - 0.5, v - 0.5);
    let (i0, j0) = (u.floor() as i32, v.floor() as i32);
    let (a, b) = (u - i0 as f32, v - j0 as f32);
    let (t00, t10, t01, t11) = (at(i0, j0), at(i0 + 1, j0), at(i0, j0 + 1), at(i0 + 1, j0 + 1));
    let mut out = [0.0; 4];
    for c in 0..4 {
        out[c] = (1.0 - a) * (1.0 - b) * t00[c] + a * (1.0 - b) * t10[c] + (1.0 - a) * b * t01[c] + a * b * t11[c];
    }
    out
}

// ---- the interpreter ----------------------------------------------------------

fn uniforms(c: &Code<'_>, size: [u32; 4]) -> Vec<u32> {
    c.uniforms
        .iter()
        .map(|u| match u.contents {
            0 | 40 | 41 => u.data,
            1 => 0,
            2..=5 => 1.0f32.to_bits(),
            44..=47 if u.data == 0 => size[(u.contents - 44) as usize],
            0x1_0000 => (-1.0f32).to_bits(),
            c => panic!("uniform contents {c} (data {}) not modelled by this test", u.data),
        })
        .collect()
}

/// `o = texture(t, v.xy)` (`v.xyz` for a cube) at each of `pts` on the
/// interpreter: the colour, from the f16 words the fragment writes.
fn sample(tex: &Texture, sampler: Sampler, pts: &[[f32; 3]]) -> Vec<[f32; 4]> {
    let (ty, e) = if tex.cube { ("samplerCube", "v.xyz") } else { ("sampler2D", "v.xy") };
    let fs = format!(
        "#version 300 es\nprecision highp float;\nuniform highp {ty} t;\nin highp vec4 v;\nout vec4 o;\n\
         void main() {{ o = texture(t, {e}); }}\n"
    );
    const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                      void main() { v = a_pos; gl_Position = a_pos; }\n";
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(&fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    let (w, h) = tex.size();
    let u = uniforms(&p.fs.code, [w, h, 1, 1]);
    pts.iter()
        .map(|pt| {
            let one = [1.0f32.to_bits(); LANES];
            let z = [0u32; LANES];
            let vary = [pt[0], pt[1], pt[2], 1.0];
            let varyings = vary.iter().map(|c| sim::Varying { vary: [c.to_bits(); LANES], c: z }).collect();
            let inputs = FsInputs { w: one, w_centroid: one, z, x: z, y: z, back_facing: [false; LANES], varyings };
            let env =
                Env { textures: vec![tex.clone()], samplers: vec![sampler], threads: p.fs.threads, ..Env::default() };
            let o = sim::run(p.fs.code.words, &u, &Inputs::Fragment(Box::new(inputs)), &env)
                .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
            let word = |k: usize| o.tlb[k].value[0].expect("a defined value");
            let hf = |x: u32| sim::half_to_f32(x as u16);
            [hf(word(0) & 0xffff), hf(word(0) >> 16), hf(word(1) & 0xffff), hf(word(1) >> 16)]
        })
        .collect()
}

const NEAREST: Sampler = Sampler::NEAREST_CLAMP;
const LINEAR: Sampler =
    Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };

fn within(filter: Filter) -> MinFilter {
    match filter {
        Filter::Nearest => MinFilter::Nearest,
        Filter::Linear => MinFilter::Linear,
    }
}

/// Channels the class defines: R16F's red only (the rest is the TMU's
/// constant, which dEQP's R11 reference does not compare either).
fn channels(class: Class) -> usize {
    if class == Class::R16F {
        1
    } else {
        4
    }
}

fn err(got: [f32; 4], want: [f32; 4], n: usize) -> f32 {
    (0..n).map(|c| (got[c] - want[c]).abs()).fold(0.0, f32::max)
}

/// The largest error over `pts`.
fn max_error(class: Class, texels: &[[f32; 4]], (w, h): (u32, u32), sp: Sampler, pts: &[[f32; 2]]) -> f32 {
    let tex = tex_of(class, w, h, texels, false, 1);
    let p3: Vec<[f32; 3]> = pts.iter().map(|p| [p[0], p[1], 0.0]).collect();
    let got = sample(&tex, sp, &p3);
    got.iter()
        .zip(pts)
        .map(|(g, pt)| err(*g, reference(texels, w as i32, h as i32, sp, pt[0], pt[1], class), channels(class)))
        .fold(0.0, f32::max)
}

fn centres(w: u32, h: u32) -> Vec<[f32; 2]> {
    (0..h).flat_map(|j| (0..w).map(move |i| [(i as f32 + 0.5) / w as f32, (j as f32 + 0.5) / h as f32])).collect()
}

/// Points between texel centres (LINEAR blends all four taps).
fn between(w: u32, h: u32) -> Vec<[f32; 2]> {
    let mut v = Vec::new();
    for j in 0..h * 2 {
        for i in 0..w * 2 {
            v.push([(i as f32 + 0.37) / (w * 2) as f32, (j as f32 + 0.71) / (h * 2) as f32]);
        }
    }
    v
}

/// class, signed, name, NEAREST threshold, LINEAR threshold
const CASES: [(Class, bool, &str, f32, f32); 6] = [
    (Class::R16F, false, "R11_EAC r16f", 0.0078585, 0.015702),
    (Class::R16F, true, "SIGNED_R11_EAC r16f", 0.015717, 0.031403),
    (Class::Rgba16F, false, "RG11_EAC rgba16f", 0.0078585, 0.015702),
    (Class::Rgba16F, true, "SIGNED_RG11_EAC rgba16f", 0.015717, 0.031403),
    (Class::Rgba8, false, "PUNCHTHROUGH rgba8", 2.0 / 255.0, 0.015702),
    (Class::Srgb8A8, false, "SRGB PUNCHTHROUGH srgb8_alpha8", 0.015702, 0.015702),
];

/// NEAREST at the texel centres of a 4x4 and LINEAR between them, each
/// class, against the CPU reference.
#[test]
fn decoded_classes_sample_within_deqps_thresholds() {
    for (class, signed, name, tn, tl) in CASES {
        let texels = texels_of(class, signed, 16);
        let en = max_error(class, &texels, (4, 4), NEAREST, &centres(4, 4));
        let el = max_error(class, &texels, (4, 4), LINEAR, &between(4, 4));
        println!("{name:34} 4x4 NEAREST max {en:.6} (threshold {tn:.6})  LINEAR max {el:.6} (threshold {tl:.6})");
        assert!(en <= tn, "{name} NEAREST {en} > {tn}");
        assert!(el <= tl, "{name} LINEAR {el} > {tl}");
    }
}

/// A 3x5 texture (not a power of two) under REPEAT and MIRRORED_REPEAT,
/// coordinates from about -1.3 to 2.7 on each axis, NEAREST and LINEAR.
#[test]
fn a_non_power_of_two_texture_wraps() {
    let pts: Vec<[f32; 2]> = (0..12)
        .flat_map(|j| (0..12).map(move |i| [-1.3 + i as f32 * 0.337 + 0.011, -1.3 + j as f32 * 0.341 + 0.007]))
        .collect();
    for (class, signed, name, tn, tl) in CASES {
        let texels = texels_of(class, signed, 15);
        for mode in [Wrap::Repeat, Wrap::MirroredRepeat] {
            for (filter, thr) in [(Filter::Nearest, tn), (Filter::Linear, tl)] {
                let sp = Sampler { mag: filter, min: within(filter), wrap_s: mode, wrap_t: mode };
                let e = max_error(class, &texels, (3, 5), sp, &pts);
                println!("{name:34} 3x5 {mode:?} {filter:?} max {e:.6} (threshold {thr:.6})");
                assert!(e <= thr, "{name} {mode:?} {filter:?}: {e} > {thr}");
            }
        }
    }
}

/// A cube of 2x2 faces: the direction through each face texel's centre
/// reads that face's texel (NEAREST).
#[test]
fn a_cube_face_reads_its_texels() {
    for (class, signed, name, tn, _) in CASES {
        let texels = texels_of(class, signed, 24);
        let tex = tex_of(class, 2, 2, &texels, true, 6);
        let mut pts = Vec::new();
        let mut want = Vec::new();
        for (f, face) in Face::ALL.into_iter().enumerate() {
            for j in 0..2 {
                for i in 0..2 {
                    let (sc, tc) = (i as f32 - 0.5, j as f32 - 0.5);
                    let (x, y, z) = texref::face_direction(face, sc, tc);
                    pts.push([x, y, z]);
                    want.push(truth(class, texels[f * 4 + j * 2 + i]));
                }
            }
        }
        let got = sample(&tex, NEAREST, &pts);
        let worst = got.iter().zip(&want).map(|(g, w)| err(*g, *w, channels(class))).fold(0.0, f32::max);
        println!("{name:34} cube NEAREST max {worst:.6} (threshold {tn:.6})");
        assert!(worst <= tn, "{name} cube: {worst} > {tn}");
    }
}
