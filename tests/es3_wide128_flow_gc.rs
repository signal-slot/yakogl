// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the plane join of a keyed unit inside control flow
//! and on partly used results (item gc-w128-flow,
//! docs/notes/gc-128bit-texture-sampling.md, docs/notes/gc-128bit-swizzle.md
//! section 5.2).
//!
//! Through the compiler entry the GL core uses (`wide128_variant`, with a
//! non-empty key), pinned for a look-up
//!
//! - inside an `if`/`else` on a uniform and inside a `for` loop with a
//!   uniform bound: every look-up on the keyed unit reads both slots;
//! - in a user function called twice, and two look-ups of one keyed sampler
//!   at different coordinates: each look-up is split on its own;
//! - whose result is used only in part (`.x`, `.w`, `.zw`): which plane
//!   look-ups the code keeps;
//!
//! and that the plain shader and the keyed one stay two records. The
//! texelFetch/textureLod/offset cases and several sampler uniforms are
//! tests/es3_wide128_texfetch_gc.rs and tests/es3_wide128_uniforms_gc.rs.
//! Inert: it needs no flip, no GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_flow_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::online::vivante::wide128::compile_planes;
use yakogl::shaderc::{
    release_shader, wide128_planes, wide128_sensitivity, wide128_variant, LodKey, ShcShader, Wide128Planes,
    SHC_STAGE_FRAGMENT,
};

/// Held across every compile and every read of its code: the plane key is
/// process-wide, and `forget_compiled` resets the arena the code lives in.
static COMPILING: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    g
}

fn text(log: &[core::ffi::c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(src: &str) -> ShcShader {
    let src = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(SHC_STAGE_FRAGMENT, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "{}", text(&log));
    sh
}

fn variant(sh: &ShcShader, planes: Wide128Planes) -> ShcShader {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let rc = wide128_variant(sh, planes, LodKey::NONE, &mut out, &mut log);
    assert_eq!(rc, 0, "{}", text(&log));
    out
}

/// The `TEX_ID` of every look-up of `sh`, in program order.
fn units(sh: &ShcShader) -> Vec<u32> {
    sh.usc_words()
        .chunks(4)
        .map(|c| decode_instr(&c.try_into().unwrap()).expect("decodes"))
        .filter(|i| i.op().name.starts_with("tex") || i.op().name == "txf")
        .map(|i| i.get("TEX_ID").expect("a look-up names its unit"))
        .collect()
}

fn listing(sh: &ShcShader) -> Vec<String> {
    sh.usc_words()
        .chunks(4)
        .map(|c| {
            let mut s = String::new();
            yakogl::shaderc::online::vivante::listing::body_text(
                &mut s,
                &decode_instr(&c.try_into().unwrap()).expect("decodes"),
            )
            .expect("text");
            s
        })
        .collect()
}

fn count(u: &[u32], unit: u32) -> usize {
    u.iter().filter(|x| **x == unit).count()
}

fn fs(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nin vec2 uv;\nin vec2 uv2;\nuniform int n;\n\
         uniform highp sampler2D s;\n{decl}\nout highp vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

const COMPANION: u32 = 5;

/// The plain shader, its keyed variant (companion slot 5 for binding 0, the
/// alpha-carrying class), and the checks that hold for every case: the
/// plain code is untouched, the key is put back, and the variant is a
/// record of its own that the empty key and a plain recompile never reach.
fn keyed(src: &str) -> (ShcShader, ShcShader, Vec<u32>, Vec<u32>) {
    let sh = compile(src);
    let sense = wide128_sensitivity(&sh);
    assert_eq!(sense & 1, 1, "binding 0 is sampled");
    let key = wide128_planes(sense, &[Some(COMPANION as u8)]);
    assert_eq!(key, Wide128Planes::NONE.with(0, COMPANION as u8));
    let plain = units(&sh);
    let v = variant(&sh, key);
    assert!(compile_planes().is_empty(), "the key is put back");
    assert_ne!(v.priv_, sh.priv_, "the keyed shader is its own record");
    assert_eq!(variant(&sh, key).priv_, v.priv_, "the same key finds the same record");
    let empty = variant(&sh, Wide128Planes::NONE);
    assert_eq!(empty.priv_, sh.priv_, "the empty key is the plain record");
    assert_eq!(empty.usc_words(), sh.usc_words());
    assert_eq!(units(&sh), plain, "the plain code is untouched");
    assert_eq!(compile(src).usc_words(), sh.usc_words(), "a plain recompile never gets the split code");
    release_shader(&empty);
    let split = units(&v);
    (sh, v, plain, split)
}

/// Every look-up on unit 0 of the plain code has a partner on the
/// companion in the keyed code, and no look-up of another unit appears.
fn every_lookup_reads_both(plain: &[u32], split: &[u32]) {
    assert!(!plain.is_empty());
    assert!(plain.iter().all(|u| *u == 0), "{plain:?}");
    assert_eq!(count(split, 0), plain.len(), "{split:?}");
    assert_eq!(count(split, COMPANION), plain.len(), "{split:?}");
    assert_eq!(split.len(), 2 * plain.len(), "{split:?}");
}

/// The joined lanes of a full-vec4 use: the second plane's x,y land in z,w.
fn joined_zw(sh: &ShcShader, times: usize) {
    let l = listing(sh);
    let n = l.iter().filter(|t| t.starts_with("mov") && t.contains(".__zw")).count();
    assert!(n >= times, "{times} joins into z,w, found {n}: {l:#?}");
}

#[test]
fn a_lookup_in_an_if_else_reads_both_slots() {
    let _g = serial();
    let src = fs("uniform bool b;", "if (b) { o = texture(s, uv); } else { o = texture(s, uv2) * 2.0; }");
    let (sh, v, plain, split) = keyed(&src);
    assert_eq!(plain.len(), 2, "one look-up per arm: {plain:?}");
    every_lookup_reads_both(&plain, &split);
    joined_zw(&v, 2);
    assert!(v.usc_words().len() > sh.usc_words().len());
    release_shader(&v);
    release_shader(&sh);
}

#[test]
fn a_lookup_in_a_loop_with_a_uniform_bound_reads_both_slots() {
    let _g = serial();
    let src =
        fs("", "vec4 a = vec4(0.0); for (int i = 0; i < n; i++) { a += texture(s, uv + vec2(float(i))); } o = a;");
    let (sh, v, plain, split) = keyed(&src);
    assert_eq!(plain, [0], "the loop body is one look-up, not unrolled for a uniform bound");
    every_lookup_reads_both(&plain, &split);
    joined_zw(&v, 1);
    release_shader(&v);
    release_shader(&sh);
}

#[test]
fn a_user_function_called_twice_splits_each_call() {
    let _g = serial();
    let src = fs("vec4 fetch(vec2 p) { return texture(s, p) + vec4(1.0); }", "o = fetch(uv) + fetch(uv2);");
    let (sh, v, plain, split) = keyed(&src);
    assert_eq!(plain.len(), 2, "the function is inlined at each call: {plain:?}");
    every_lookup_reads_both(&plain, &split);
    joined_zw(&v, 2);
    release_shader(&v);
    release_shader(&sh);
}

#[test]
fn two_lookups_at_different_coordinates_are_two_pairs() {
    let _g = serial();
    let src = fs("", "o = texture(s, uv) - texture(s, uv2);");
    let (sh, v, plain, split) = keyed(&src);
    assert_eq!(plain, [0, 0]);
    every_lookup_reads_both(&plain, &split);
    joined_zw(&v, 2);
    assert_eq!(split, [0, COMPANION, 0, COMPANION], "each pair is the unit then the companion, not interleaved");
    release_shader(&v);
    release_shader(&sh);
}

/// A partly used result: the code keeps exactly the plane look-ups whose
/// lanes are read (the dead one is dropped after the join).
#[test]
fn a_partly_used_result_keeps_the_planes_it_reads() {
    let _g = serial();
    for (body, lo, hi) in [
        ("o = vec4(texture(s, uv).x);", true, false),
        ("o = vec4(texture(s, uv).w);", false, true),
        ("o = vec4(texture(s, uv).zw, 0.0, 1.0);", false, true),
        ("o = vec4(texture(s, uv).xy, 0.0, 1.0);", true, false),
    ] {
        let src = fs("", body);
        let (sh, v, plain, split) = keyed(&src);
        assert_eq!(plain, [0], "{body}");
        eprintln!("{body}: plain {plain:?} keyed {split:?}");
        assert!(split.iter().all(|u| *u == 0 || *u == COMPANION), "{body}: {split:?}");
        assert_eq!(count(&split, 0), lo as usize, "{body}: the R,G plane look-up is kept only when read: {split:?}");
        assert_eq!(
            count(&split, COMPANION),
            hi as usize,
            "{body}: the B,A plane look-up is kept only when read: {split:?}"
        );
        release_shader(&v);
        release_shader(&sh);
    }
}
