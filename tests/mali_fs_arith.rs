// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's fragment arithmetic (the ARITH family of plan #367) on the
//! Mali, against the Mesa 26.2.1 drm-shim references `tools/mali/testdata/
//! gm2_fsmisc_1..8` (`sqrt`, `fract`, `clamp`, `smoothstep`, `exp2`, `mix`
//! of constants, `mix` of varyings, `step`) and `gm2_fsarith_1..2` (the lit
//! shading of glmark2's shading and bump scenes: `normalize`, a `mat4`
//! product, `max(dot(), 0.0)`, `pow`), byte for byte
//! (docs/notes/mali-fs-arith.md).
//!
//! The gm2_fsmisc pairs are built as glCompileShader and glLinkProgram
//! build them (`tests/mali_ref/mod.rs`) and compared in every region. The
//! gm2_fsarith pairs' link is refused by the varying-link family (their
//! vertex shader writes a `vec2` the fragment shader does not read; see the
//! note's section 4), so their fragment program is compiled alone, as
//! glCompileShader makes it, and compared with the reference's
//! `fs.fragment`: the words, the facts and the FAU words, every uniform 0
//! (the references were drawn without gl_ref.py's `--uniform-values`); which
//! uniform word each pushed FAU word holds is pinned from the program's own
//! reads (`gm2_fsarith_*_push_the_columns_mesa_reads`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_fs_arith
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

use core::ffi::c_char;
use std::sync::{Mutex, MutexGuard};
use yakogl::shaderc::online::mali::bir::Sysval;
use yakogl::shaderc::online::mali::fau::FauWord;
use yakogl::shaderc::online::mali::MaliShader;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// One test at a time: the compiler's scratch arena and record cache are
/// process-wide ([`mali_ref::compile_pair`] locks its own calls only), and
/// [`compile_fs`] resets them, so each test holds this lock over its compiles
/// and the reads of their results (`src/test_rules.rs`, the third family).
static COMPILING: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    COMPILING.lock().unwrap_or_else(|p| p.into_inner())
}

/// A fragment shader compiled alone (glCompileShader): its words without
/// the zero padding, `work_reg_count`, the preload and the FAU, or the
/// compile log.
fn compile_fs(src: &str) -> Result<(Vec<u64>, u8, u64, Vec<FauWord>), String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 4096];
    let z = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    yakogl::shaderc::online::backend::forget_compiled();
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(SHC_STAGE_FRAGMENT, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect());
    }
    let mut code: Vec<u64> = out.usc_words().chunks(2).map(|c| c[0] as u64 | (c[1] as u64) << 32).collect();
    while code.last() == Some(&0) {
        code.pop();
    }
    let m = MaliShader::of(&out).expect("a Mali record");
    Ok((code, m.work_reg_count, m.preload, m.fau.to_vec()))
}

/// The FAU words for a draw that sets no uniform (all 0).
fn fau_unset(fau: &[FauWord]) -> Vec<u32> {
    let mut v: Vec<u32> = fau
        .iter()
        .map(|w| match *w {
            FauWord::Sysval(Sysval::ViewportScale | Sysval::ViewportOffset, _) => 0x4200_0000,
            FauWord::Constant(c) => c,
            FauWord::Uniform { .. } | FauWord::UniformHalves { .. } | FauWord::BlendConstant(_) => 0,
            other => panic!("a FAU word this test does not model: {other:?}"),
        })
        .collect();
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// A list of hex words (`key`) of the reference's `region`.
fn hex_list(json: &str, region: &str, key: &str) -> Vec<u64> {
    let r = &json[json.find(&format!("\"{region}\"")).unwrap_or_else(|| panic!("{region}"))..];
    let f = &r[r.find(&format!("\"{key}\":")).expect("a list")..];
    let list = &f[f.find('[').unwrap() + 1..f.find(']').unwrap()];
    list.split(',').map(|x| u64::from_str_radix(x.trim().trim_matches('"'), 16).expect("a hex word")).collect()
}

/// A number field of the reference's `region`.
fn field(json: &str, region: &str, key: &str) -> u64 {
    let r = &json[json.find(&format!("\"{region}\"")).unwrap_or_else(|| panic!("{region}"))..];
    let f = r[r.find(&format!("\"{key}\":")).expect("a field") + key.len() + 3..].trim_start();
    let t: String = f.trim_start_matches('"').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    match t.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).expect("hex"),
        None => t.parse().expect("a number"),
    }
}

/// `name`'s fragment shader compiled alone equals the reference's
/// `fs.fragment`: words, `work_reg_count`, preload and FAU words (the
/// uniforms 0). Returns the FAU.
fn assert_fs_matches(name: &str) -> Vec<FauWord> {
    let r = mali_ref::Reference::load(name);
    let (code, wrc, preload, fau) = compile_fs(&r.fs).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
    let words = hex_list(&r.json, "fs.fragment", "words");
    let mut d: Vec<String> = mali_ref::word_diff("fs.fragment", &code, &words).into_iter().collect();
    if u64::from(wrc) != field(&r.json, "fs.fragment", "work_reg_count") {
        d.push(format!("work_reg_count ours {wrc}"));
    }
    if preload != field(&r.json, "fs.fragment", "preload") {
        d.push(format!("preload ours {preload:#x}"));
    }
    let theirs: Vec<u32> = hex_list(&r.json, "fs.fragment", "fau").into_iter().map(|w| w as u32).collect();
    let ours = fau_unset(&fau);
    if ours != theirs {
        d.push(format!("FAU words ours {ours:08x?}, reference {theirs:08x?}"));
    }
    assert!(d.is_empty(), "{name}: differs from its reference:\n{}", d.join("\n"));
    fau
}

/// `sqrt(x)` of a vec4 varying: each lane's `FRSQ.f16`, then each lane's
/// `FRCP.f16` (`lower_fsqrt` of the vector), the high halves read `.h11`.
#[test]
fn gm2_fsmisc_1_sqrt_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_1");
}

/// `fract(3.0 * x.x)` squared beside `x.yzw`: the product read twice is
/// not fused, the word `(d, x.y)` a `MKVEC.v2i16` of the lane and the
/// varying's half.
#[test]
fn gm2_fsmisc_2_fract_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_2");
}

/// `clamp(x, 0.25, 0.75)`: `FMAX.v2f16` then `FMIN.v2f16` of the pairs.
#[test]
fn gm2_fsmisc_3_clamp_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_3");
}

/// `vec4(smoothstep(0.25, 0.75, x.x))`: `(x - 0.25) * 2.0` as `fadd(a, a)`
/// with the saturate fused, `t * (t * (3.0 - 2.0 t))`, the word one lane in
/// both halves.
#[test]
fn gm2_fsmisc_4_smoothstep_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_4");
}

/// `exp2(x)` of a vec4 varying: at fp32 on the load's words
/// (`FMA_RSCALE.f32`, `F32_TO_S32` with no rounding mode, `FEXP.f32`), the
/// results narrowed in pairs.
#[test]
fn gm2_fsmisc_5_exp2_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_5");
}

/// `exp2(-2.0 * d * d)` of `d = min(..)`, then `mix` of two constants:
/// `dist.x * dist.w` and `dist.y * dist.w` one pair (`nir_opt_vectorize`),
/// `-2.0 * d` as `fneg(fadd(d, d) * d)`, `exp2` widened and narrowed, the
/// constants folded at fp16.
#[test]
fn gm2_fsmisc_6_exp2_mix_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_6");
}

/// `mix(a, b, t)` of three varyings: `t` converted alone, first in the
/// `ffma`.
#[test]
fn gm2_fsmisc_7_mix_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_7");
}

/// `step(0.5, x) * y`: one `FCMP_OR.v2f16.ge.m1` a pair, its mask read
/// whole by `MUX.v2i16 0, y, mask`.
#[test]
fn gm2_fsmisc_8_step_matches_its_reference() {
    let _one = one_at_a_time();
    mali_ref::assert_matches("gm2_fsmisc_8");
}

/// The mediump directional light: `nm * vec4(n, 1.0)` reassociated
/// (`c2 z + (c0 x + (c1 y + c3))`), its `(x, y)` one pair the dot product
/// reads by halves and its `z` an operation alone, `normalize` of a vec3
/// uniform, `max(dot(), 0.0)` as `.clamp_0_inf`, `pow(.., 100.0)` as
/// `bi_fpow_32`, the colour's `(x, y)` a pair and `z` alone beside 1.0.
#[test]
fn gm2_fsarith_1_lit_matches_its_reference() {
    let _one = one_at_a_time();
    assert_fs_matches("gm2_fsarith_1");
}

/// The same at highp: each lane's matrix chain whole, the fp32 `pow`.
#[test]
fn gm2_fsarith_2_lit_highp_matches_its_reference() {
    let _one = one_at_a_time();
    assert_fs_matches("gm2_fsarith_2");
}

/// The uniforms the lit programs push, read from the instructions that
/// use them (the reference's FAU holds 0 for each): `light` at dwords 0-2,
/// `half_v` at 4-6, `nm` at 8-23 (column c at 8 + 4c). gm2_fsarith_1's
/// first `FMA.v2f16 r2, u2.w0, r0.h11, u2.w1` is `nm[1].xy * n.y +
/// nm[3].xy`, so `u2.w0` holds dwords 12-13 and `u2.w1` 20-21; `u4` is
/// `light`, `u6.w0` `nm[0].xy`, `u7.w0` `nm[2].xy`. At highp each word is
/// one dword: `FMA.f32 r3, u2.w0, r1, u2.w1` is `nm[1].x * n.y + nm[3].x`.
#[test]
fn gm2_fsarith_push_the_columns_mesa_reads() {
    let _one = one_at_a_time();
    let fau = assert_fs_matches("gm2_fsarith_1");
    let halves = |lo: u16| FauWord::UniformHalves { lo, hi: Some(lo + 1) };
    for (w, want) in [(4, halves(12)), (5, halves(20)), (8, halves(0)), (12, halves(8)), (14, halves(16))] {
        assert_eq!(fau.get(w), Some(&want), "gm2_fsarith_1 FAU word {w}: {fau:?}");
    }
    let fau = assert_fs_matches("gm2_fsarith_2");
    for (w, d) in [(4, 12), (5, 20), (16, 8), (19, 16)] {
        assert_eq!(fau.get(w), Some(&FauWord::Uniform { dword: d }), "gm2_fsarith_2 FAU word {w}: {fau:?}");
    }
}

/// The shapes past what the references draw stay refused: `pow` by an
/// exponent nir_opt_algebraic rewrites (2.0: `a * a`), `exp2` of a
/// varying's lane alone beside a constant, a highp `log2` outside a `pow`,
/// and a `mat3` uniform of a mediump fragment shader.
#[test]
fn the_shapes_no_reference_draws_stay_refused() {
    let _one = one_at_a_time();
    let refused = |src: &str, text: &str| match compile_fs(src) {
        Ok(_) => panic!("{src}: compiles"),
        Err(e) => assert!(e.contains(text), "{src}: {e}"),
    };
    refused(
        "precision mediump float; varying vec4 x; void main() { gl_FragColor = vec4(pow(max(x.x, 0.0), 2.0)); }",
        "Mesa's algebraic rules",
    );
    refused(
        "precision mediump float; varying vec4 x; void main() { gl_FragColor = vec4(exp2(x.x), 0.5, 0.5, 1.0); }",
        "gl_FragColor lane other than",
    );
    refused(
        "precision highp float; varying vec4 x; void main() { gl_FragColor = vec4(log2(x.x)); }",
        "arithmetic other than",
    );
    refused(
        "precision mediump float; uniform mat3 m; varying vec3 v; void main() { gl_FragColor = vec4(m * v, 1.0); }",
        "uniform other than",
    );
}
