// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the GL core's entry to the 128-bit plane-join
//! compile (item wide128-shc-entry, docs/notes/gc-128bit-texture-sampling.md).
//!
//! The call contract of `online::backend::wide128_sensitivity`, driven
//! through the free functions of `yakogl::shaderc` as the GL core calls
//! them, on shaders the online back end compiled:
//!
//! - the sensitivity names the bindings a `sampler2D` / `usampler2D`
//!   look-up reads, and not a `textureSize`'s;
//! - an empty key is the shader as today (a copy of it);
//! - a key gives the stage compiled again with the look-up on the keyed
//!   binding split into two, the second on the companion slot, in both
//!   stages;
//! - the compile key is put back after the call, a refusal included, and
//!   the plain shader and the split one are two records: a plain compile of
//!   the same source never gets the split code.
//!
//! - a `usampler2D` and an `isampler2D` join as two look-ups and moves, with
//!   no float conversion of the words;
//! - a GL texture swizzle leaves CONFIG1 of a 128-bit class at the identity
//!   (a pure `format_words` call), so both plane descriptors carry it;
//! - wrap R of a 3D RGBA32F binding is the W field of the slot's SAMP_CTRL0
//!   (and the companion's, once the class is sampled), S and T their own.
//!
//! The section 5.1 pins of docs/notes/gc-128bit-swizzle.md, through the GL
//! core and the Vivante back end on a stand-in OSAL (item gc-w128-tests),
//! now that GC_SAMPLED_TEXFMTS holds RGBA32F/UI/I (8f019dae):
//!
//! - flip pins: the mask holds the three classes, and a GL_RGBA32UI,
//!   GL_RGBA32I and GL_RGB32F texture drawn through a sampling shader gets
//!   the descriptor pair (own slot and companion slot 1, both G32R32, level 0
//!   of plane 1 one plane of 8-byte texels after plane 0's);
//! - the guard (b6d7a4d1): a two-plane binding whose GL texture swizzle the
//!   planes cannot join (R<->B, R<->G, ZERO or ONE on a lane, alpha moved)
//!   is refused -- no kick, GL_NO_ERROR, the text 'texture swizzle is not
//!   joined' (read in a child process: only a process's first refusal is
//!   logged) -- and the explicit (RED, GREEN, BLUE, ALPHA) draws;
//! - no_alpha: GL_RGB32F/I/UI store alpha 1 (1.0f or 1), draw as two planes
//!   with A = ONE in both descriptors, also under a swizzle that leaves alpha
//!   alone, and are refused with alpha ZERO; whether the draw's key carries
//!   `no_alpha` is said on stderr (today it does not: `.w` is the stored 1);
//! - both planes' descriptor words of a 2D RGBA32I binding (CONFIG0/1/2,
//!   size, stride, slice, levels, the tiled plane texels, SAMP_CTRL0/1);
//! - wrap R on a 3D RGBA32UI/I binding: the W field of both slots' SAMP_CTRL0;
//! - isampler2D / usampler2D draws of RGBA32I / RGBA32UI (texture with
//!   integer arithmetic, texelFetch): drawn, both planes bound.
//!
//! GL_RGBA32F is stored in the 128-bit class on this back end
//! (`sized_format` / `rgba32f_own_class`, src/gl/texture.rs), so its rows
//! are two planes as the other formats' are.
//!
//! What it cannot see: the GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::online::vivante::wide128::compile_planes;
use yakogl::shaderc::{
    release_shader, wide128_planes, wide128_sensitivity, wide128_variant, LodKey, ShcShader, Wide128Planes,
    SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
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

/// The shader object's compile of `src`, as the GL core makes it.
fn compile(stage: u32, src: &str) -> ShcShader {
    let src = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "{}", text(&log));
    sh
}

/// `sh` under `planes` (and no sampler-state key), or the refusal's log.
fn variant(sh: &ShcShader, planes: Wide128Planes) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    match wide128_variant(sh, planes, LodKey::NONE, &mut out, &mut log) {
        0 => Ok(out),
        rc => {
            assert!(rc < 0, "{rc}");
            Err(text(&log))
        }
    }
}

/// The `TEX_ID` of every look-up of `sh`, in program order.
fn units(sh: &ShcShader) -> Vec<(&'static str, u32)> {
    sh.usc_words()
        .chunks(4)
        .map(|c| decode_instr(&c.try_into().unwrap()).expect("decodes"))
        .filter(|i| i.op().name.starts_with("tex") || i.op().name == "txf")
        .map(|i| (i.op().name, i.get("TEX_ID").expect("a look-up names its unit")))
        .collect()
}

fn fs(decl: &str, out: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nin vec2 uv;\n{decl}\nout highp {out} o;\nvoid main() {{ {body} }}\n"
    )
}

/// The sensitivity: bit `b` for binding `b` sampled by a float or an
/// unsigned look-up; nothing for a `textureSize` or no look-up; and the draw's key of it names only the sampled bindings that
/// hold two planes.
#[test]
fn the_sensitivity_names_the_bindings_a_lookup_reads() {
    let _g = serial();
    let both = compile(
        SHC_STAGE_FRAGMENT,
        &fs(
            "uniform highp sampler2D a;\nuniform highp usampler2D b;",
            "vec4",
            "o = texture(a, uv) + vec4(texture(b, uv));",
        ),
    );
    let mut u = units(&both);
    u.sort();
    assert_eq!(u, [("texld", 0), ("texld", 1)]);
    assert_eq!(wide128_sensitivity(&both), 0b11);
    let unsigned = compile(SHC_STAGE_FRAGMENT, &fs("uniform highp usampler2D b;", "uvec4", "o = texture(b, uv);"));
    assert_eq!(wide128_sensitivity(&unsigned), 0b1);
    /* a shadow look-up (not reported: it reads a depth texture) is not
     * asked here: the GC7000 back end does not select one yet, so no
     * record of it exists to ask */
    let size = compile(
        SHC_STAGE_FRAGMENT,
        &fs("uniform highp sampler2D s;", "vec4", "o = vec4(vec2(textureSize(s, 0)), 0.0, 1.0);"),
    );
    assert_eq!(wide128_sensitivity(&size), 0, "textureSize reads the primary unit");
    let none = compile(SHC_STAGE_FRAGMENT, &fs("", "vec4", "o = vec4(uv, 0.0, 1.0);"));
    assert_eq!(wide128_sensitivity(&none), 0);
    assert_eq!(wide128_sensitivity(&ShcShader::ZERO), 0, "a shader the compiler did not make");

    /* the draw's key: binding 1 is two planes (B,A at slot 7), binding 0
     * is not, binding 2 is but is never sampled */
    let key = wide128_planes(0b11, &[None, Some(7), Some(9)]);
    assert_eq!(key, Wide128Planes::NONE.with(1, 7));
    assert!(wide128_planes(0b11, &[None, None]).is_empty());
    for sh in [both, unsigned, size, none] {
        release_shader(&sh);
    }
}

/// An empty key is the shader as today: a copy of it, the same record and
/// the same code, which the caller releases like any variant.
#[test]
fn an_empty_key_is_the_shader_as_today() {
    let _g = serial();
    let sh = compile(SHC_STAGE_FRAGMENT, &fs("uniform highp sampler2D s;", "vec4", "o = texture(s, uv);"));
    let v = variant(&sh, Wide128Planes::NONE).expect("a copy");
    assert_eq!(v.priv_, sh.priv_, "the same record");
    assert_eq!(v.usc_words(), sh.usc_words());
    assert_eq!(units(&v), [("texld", 0)]);
    release_shader(&v);
    /* the input is still held: its code is still there */
    assert_eq!(units(&sh), [("texld", 0)]);
    release_shader(&sh);
}

/// A key compiles the stage again with the keyed binding's look-up as two,
/// the second on the companion slot; a float and an unsigned sampler in the
/// fragment stage, and an explicit-level look-up in the vertex stage.
#[test]
fn a_key_splits_the_lookup_onto_the_companion_slot() {
    let _g = serial();
    for (sampler, out) in [("sampler2D", "vec4"), ("usampler2D", "uvec4")] {
        let sh = compile(SHC_STAGE_FRAGMENT, &fs(&format!("uniform highp {sampler} s;"), out, "o = texture(s, uv);"));
        let sense = wide128_sensitivity(&sh);
        assert_eq!(sense, 1, "{sampler}");
        let v = variant(&sh, wide128_planes(sense, &[Some(5)])).unwrap_or_else(|e| panic!("{sampler}: {e}"));
        assert_eq!(units(&v), [("texld", 0), ("texld", 5)], "{sampler}");
        assert!(v.usc_words().len() > sh.usc_words().len(), "{sampler}: the join costs instructions");
        assert_eq!(units(&sh), [("texld", 0)], "{sampler}: the program's own shader is untouched");
        release_shader(&v);
        release_shader(&sh);
    }
    let vs = compile(
        SHC_STAGE_VERTEX,
        "#version 300 es\nuniform highp usampler2D s;\nin vec4 p;\nflat out highp uvec4 v;\n\
         void main() { v = textureLod(s, p.xy, 0.0); gl_Position = p; }\n",
    );
    let sense = wide128_sensitivity(&vs);
    assert_eq!(sense, 1);
    let plain = units(&vs);
    assert_eq!(plain.len(), 1, "{plain:?}");
    let v = variant(&vs, wide128_planes(sense, &[Some(3)])).expect("the vertex stage splits too");
    let split = units(&v);
    assert_eq!(split.len(), 2, "{split:?}");
    assert_eq!(split[0], plain[0], "the R,G plane on the binding's own unit");
    assert_eq!(split[1].0, plain[0].0, "the B,A plane the same look-up");
    assert_eq!(split[1].1, plain[0].1 + 3, "on the companion slot, numbered as the stage numbers its own");
    release_shader(&v);
    release_shader(&vs);
}

/// The compile's key is put back after the call, a refusal included, and
/// the plain shader and the split one are two records: the same key finds
/// the split record again, and a plain compile of the source never does.
#[test]
fn the_key_is_put_back_and_the_records_stay_apart() {
    let _g = serial();
    let src = fs("uniform highp sampler2D s;", "vec4", "o = texture(s, uv);");
    let sh = compile(SHC_STAGE_FRAGMENT, &src);
    let plain = sh.usc_words().to_vec();
    let key = Wide128Planes::NONE.with(0, 5);
    let v = variant(&sh, key).expect("compiles");
    assert!(compile_planes().is_empty(), "the key is put back");
    let again = variant(&sh, key).expect("compiles");
    assert_eq!(again.priv_, v.priv_, "the same key finds the same record");
    let other = variant(&sh, Wide128Planes::NONE.with(0, 6)).expect("compiles");
    assert_ne!(other.priv_, v.priv_, "another companion is another record");
    assert_eq!(units(&other), [("texld", 0), ("texld", 6)]);

    /* a plain compile of the same source: the program's own code */
    let fresh = compile(SHC_STAGE_FRAGMENT, &src);
    assert_eq!(fresh.usc_words(), &plain[..], "a plain compile never gets the split record");
    assert_ne!(fresh.priv_, v.priv_);

    /* a key the split refuses (the B,A plane on a unit the shader samples
     * itself): < 0, a log naming it, the key put back */
    let two = compile(
        SHC_STAGE_FRAGMENT,
        &fs("uniform highp sampler2D s;\nuniform highp sampler2D t;", "vec4", "o = texture(s, uv) + texture(t, uv);"),
    );
    let Err(e) = variant(&two, Wide128Planes::NONE.with(0, 1)) else {
        panic!("a companion the shader samples is refused")
    };
    assert!(e.contains("second plane is on a unit the shader samples"), "{e}");
    assert!(compile_planes().is_empty(), "the key is put back after a refusal");
    /* and the plain compile after it is still the plain one */
    let after = compile(SHC_STAGE_FRAGMENT, &src);
    assert_eq!(after.usc_words(), &plain[..]);

    for s in [v, again, other, fresh, two, after, sh] {
        release_shader(&s);
    }
}

/// The instruction listing of `sh`, one line per instruction.
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

/// A keyed binding whose texture has no alpha (RGB32 stored as RGBA32): the
/// joined `.w` is the constant 1, not the B,A plane's second channel, which
/// the descriptors' swizzle A = ONE does not reach. A binding with alpha
/// joins as before.
#[test]
fn a_no_alpha_binding_joins_a_constant_one_for_w() {
    let _g = serial();
    for (sampler, out) in [("sampler2D", "vec4"), ("usampler2D", "uvec4")] {
        let sh = compile(SHC_STAGE_FRAGMENT, &fs(&format!("uniform highp {sampler} s;"), out, "o = texture(s, uv);"));
        let alpha = Wide128Planes::NONE.with(0, 5);
        let with = variant(&sh, alpha).unwrap_or_else(|e| panic!("{sampler}: {e}"));
        let without = variant(&sh, alpha.without_alpha(0)).unwrap_or_else(|e| panic!("{sampler}: {e}"));
        assert_ne!(with.priv_, without.priv_, "{sampler}: the alpha bit is part of the record key");
        assert_eq!(units(&without), [("texld", 0), ("texld", 5)], "{sampler}");
        let (a, n) = (listing(&with), listing(&without));
        let one = if sampler == "sampler2D" { "1.0:f20" } else { "1:s20" };
        /* with alpha: the second plane's x,y become z,w, as before */
        assert!(
            a.iter().any(|t| t.starts_with("mov") && t.contains("t2.__zw, void, void, t1.xyxy")),
            "{sampler}: {a:#?}"
        );
        assert!(a.iter().all(|t| !t.contains(one)), "{sampler}: no constant: {a:#?}");
        /* without: only z comes from the plane, w is the constant */
        assert!(
            n.iter().any(|t| t.starts_with("mov") && t.contains("t2.__z_, void, void, t1.xyxy")),
            "{sampler}: {n:#?}"
        );
        assert!(
            n.iter().any(|t| t.starts_with("mov") && t.contains("t2.___w, void, void,") && t.contains(one)),
            "{sampler}: {n:#?}"
        );
        assert!(n.iter().all(|t| !t.contains(".__zw")), "{sampler}: w is not read from the plane: {n:#?}");
        assert_ne!(with.usc_words(), without.usc_words(), "{sampler}");
        /* the same key again finds its record; the key is put back */
        let again = variant(&sh, alpha.without_alpha(0)).expect("compiles");
        assert_eq!(again.priv_, without.priv_, "{sampler}");
        assert!(compile_planes().is_empty() && compile_planes().no_alpha == 0, "{sampler}: the key is put back");
        /* an alpha bit on a unit that is not keyed is no key */
        assert_eq!(alpha.without_alpha(3), alpha, "{sampler}");
        for s in [with, without, again, sh] {
            release_shader(&s);
        }
    }
}

/// An integer sampler of a 128-bit binding (`usampler2D` over RGBA32UI,
/// `isampler2D` over RGBA32I): the key splits the look-up into two `texld`s,
/// the second on the companion slot, and the result stays integer all the
/// way: the joined code is look-ups and moves, no conversion to or from float.
#[test]
fn an_integer_sampler_joins_without_a_float_conversion() {
    let _g = serial();
    for (sampler, out) in [("usampler2D", "uvec4"), ("isampler2D", "ivec4")] {
        let sh = compile(SHC_STAGE_FRAGMENT, &fs(&format!("uniform highp {sampler} s;"), out, "o = texture(s, uv);"));
        let sense = wide128_sensitivity(&sh);
        assert_eq!(sense, 1, "{sampler}");
        let v = variant(&sh, wide128_planes(sense, &[Some(4)])).unwrap_or_else(|e| panic!("{sampler}: {e}"));
        assert_eq!(units(&v), [("texld", 0), ("texld", 4)], "{sampler}: the unit and its companion");
        assert_eq!(units(&sh), [("texld", 0)], "{sampler}: the program's own shader is untouched");
        let joined = listing(&v);
        assert!(
            joined.iter().all(|t| t.starts_with("texld") || t.starts_with("mov")),
            "{sampler}: the join is look-ups and moves, no conversion of the words: {joined:#?}"
        );
        assert!(joined.iter().all(|t| !t.contains("f20")), "{sampler}: no float constant: {joined:#?}");
        assert!(
            joined.iter().any(|t| t.starts_with("mov") && t.contains(".__zw")),
            "{sampler}: the B,A plane's words land in z,w: {joined:#?}"
        );
        release_shader(&v);
        release_shader(&sh);
    }
}

/// A GL swizzle on a 128-bit binding never reaches the descriptor words: both
/// planes are the one descriptor (a byte copy but for the level addresses),
/// and a swizzle could only permute inside each plane's (r, g) pair, so
/// CONFIG1 carries the identity. A pure call of `format_words`, which
/// needs neither the mask nor a draw; the swizzle's own handling (the guard
/// that refuses it) is the flip's item.
#[test]
fn a_gl_swizzle_leaves_both_plane_descriptors_at_the_identity() {
    use yakogl::backend::*;
    use yakogl::vivante::regs::*;
    use yakogl::vivante::texture::{descriptor_swizzle, format_words, wide128_plane_format};
    let swizzles = [
        [GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED, GLES_SWIZZLE_ALPHA],
        [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED],
        [GLES_SWIZZLE_ZERO, GLES_SWIZZLE_ONE, GLES_SWIZZLE_ZERO, GLES_SWIZZLE_ONE],
        [GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED, GLES_SWIZZLE_SAME, GLES_SWIZZLE_SAME],
    ];
    let identity = [GC_SWIZZLE_R, GC_SWIZZLE_G, GC_SWIZZLE_B, GC_SWIZZLE_A];
    for format in [GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA32UI, GLES_TEXFMT_RGBA32I] {
        let plain = GlesTextureBind { format, width: 4, height: 4, mip_levels: 1, ..GlesTextureBind::ZERO };
        let own = format_words(&plain, true);
        assert_eq!(descriptor_swizzle(&wide128_plane_format(format).unwrap(), &plain), identity, "format {format}");
        for swizzle in swizzles {
            let t = GlesTextureBind { swizzle, ..plain };
            assert_eq!(
                descriptor_swizzle(&wide128_plane_format(format).unwrap(), &t),
                identity,
                "format {format} swizzle {swizzle:?}"
            );
            assert_eq!(format_words(&t, true), own, "format {format} swizzle {swizzle:?}: the same words");
            let w = format_words(&t, true)[1];
            assert_eq!(
                w & (texdesc_swizzle_r(7) | texdesc_swizzle_g(7) | texdesc_swizzle_b(7) | texdesc_swizzle_a(7)),
                texdesc_swizzle_r(GC_SWIZZLE_R)
                    | texdesc_swizzle_g(GC_SWIZZLE_G)
                    | texdesc_swizzle_b(GC_SWIZZLE_B)
                    | texdesc_swizzle_a(GC_SWIZZLE_A),
                "format {format} swizzle {swizzle:?}: CONFIG1's four fields"
            );
        }
    }
}

/* ---- the GL core's draw (item wide128-shader) ----------------------------
 *
 * The draw picks the key from the texture bound NOW (src/gl/draw.rs,
 * `wide128_select`): a usampler2D names RGBA8UI and RGBA32UI alike, and
 * only the second is two planes. These run a real draw through the GL core
 * and the Vivante back end on a stand-in OSAL (as
 * tests/es3_wide128_planes_gc.rs) and read the key the draw chose
 * (`gl::draw::wide128_draw_state`).
 *
 * RGBA32UI is in GC_SAMPLED_TEXFMTS since 8f019dae, so the RGBA32UI case
 * checks the variant for real; its closed branch (the class out of the
 * mask: no RGBA32UI texture stored, the program's shaders kept) is kept for
 * a tree without the classes. */

mod draw {
    use super::serial;
    use core::ffi::{c_char, c_void};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::sync::Mutex;
    use yakogl::backend::*;
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
    use yakogl::shaderc::Wide128Planes;
    use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
    use yakogl::vivante::regs::*;
    use yakogl::vivante::texture::{is_wide128, wide128_plane_format};

    /* the stand-in OSAL: plain memory, every kick completes at once */

    #[derive(Default)]
    struct Platform {
        allocs: BTreeMap<u64, Box<[u64]>>,
        bump: BTreeMap<u64, u64>,
        fence: i64,
        kicks: Vec<Vec<u32>>,
        faults: Vec<String>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        if size == 0 || size > 1 << 30 {
            return -12;
        }
        with_platform(|p| {
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let at = p.bump.entry(heap_base).or_insert(0);
            let gpu = heap_base + 0x1000 + *at;
            *at += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, mem);
            0
        })
    }

    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes a live descriptor.
        let m = unsafe { *mem };
        with_platform(|p| {
            if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
            }
        });
        // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
        unsafe { *mem = OsalGpuMem::ZERO };
        0
    }

    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != core::mem::size_of::<GcKick>() {
            with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
            return -22;
        }
        // SAFETY: the back end passes a live GcKick of that size.
        let k = unsafe { *(kick as *const GcKick) };
        with_platform(|p| {
            if k.magic != GC_KICK_MAGIC {
                p.faults.push(format!("submit with magic 0x{:x}", k.magic));
                return -22;
            }
            if let Some(a) = p.allocs.get(&k.stream_gpu) {
                // SAFETY: the allocation holds at least `stream_bytes` bytes.
                let words: &[u32] =
                    unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
                p.kicks.push(words.to_vec());
            }
            p.fence += 1;
            p.fence
        })
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.fence)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
        if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
            return 0;
        }
        -22
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    /// The OSAL log lines, in order (the refusal text of the swizzle guard).
    static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// The OSAL's log hook: the library only ever calls it with ("%s", line)
    /// (`osal::log_str`), so a two-argument function stands in for the
    /// variadic (as tests/es3_wide128_refusals_gc.rs).
    unsafe extern "C" fn p_log(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: `line` is the NUL-terminated string `log_str` passes.
        let s = unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy().into_owned();
        LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
    }

    // SAFETY: see p_log: the library only ever calls it with ("%s", line).
    const LOG_HOOK: LogFn =
        unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log) };

    static OSAL: OsalOps = OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(p_gpu_alloc),
        gpu_free: Some(p_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(p_cache),
        cache_invd: Some(p_cache),
        submit: Some(p_submit),
        wait: Some(p_wait),
        completed: Some(p_completed),
        reg_read: None,
        reg_write: None,
        query: Some(p_query),
        log: Some(LOG_HOOK),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// Run `case` with the Vivante back end up on the stand-in OSAL and a
    /// fresh 64x64 context, under the compile lock.
    fn on_gc7000(case: impl FnOnce()) {
        let _g = serial();
        // SAFETY: the guard serialises the tests; the tables are statics.
        let ctx = unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null(), "context pool exhausted");
            c
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
        // SAFETY: the context made above.
        unsafe {
            yakogl::gl::context::gl_context_destroy(ctx);
            yakogl::vivante::backend::gc_shutdown();
        }
        let faults = with_platform(|p| {
            /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
            for (_, a) in core::mem::take(&mut p.allocs) {
                core::mem::forget(a);
            }
            core::mem::take(&mut p.faults)
        });
        if let Err(e) = r {
            std::panic::resume_unwind(e);
        }
        assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    }

    fn error() -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glGetError() }
    }

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let text = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    }

    const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 uv;\nvoid main() { uv = p.xy; gl_Position = p; }\n";
    const FS: &str = concat!(
        "#version 300 es\nprecision highp float;\nuniform highp usampler2D s;\nin vec2 uv;\n",
        "out vec4 o;\nvoid main() { o = vec4(texture(s, uv)) / 255.0; }\n"
    );

    /// The program both cases draw: a usampler2D read by `texture`.
    fn usampler_program() -> GLuint {
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            p
        }
    }

    /// A 2x2 integer texture of `internal`, NEAREST, bound to unit 0; the
    /// error of its glTexImage2D.
    fn uint_texture(internal: GLenum) -> GLenum {
        let ty = if internal == GL_RGBA32UI { GL_UNSIGNED_INT } else { GL_UNSIGNED_BYTE };
        let texels = [0x5au32; 16];
        // SAFETY: plain GL calls; `texels` holds 2x2 texels of either size.
        unsafe {
            let mut t: GLuint = 0;
            glActiveTexture(GL_TEXTURE0);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal as GLint,
                2,
                2,
                0,
                GL_RGBA_INTEGER,
                ty,
                texels.as_ptr() as *const c_void,
            );
        }
        error()
    }

    static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

    /// One quad with program `p` sampling unit 0, drawn and finished; the
    /// plane sensitivity and key the draw worked out.
    fn draw(p: GLuint) -> ([u32; 2], [Wide128Planes; 2]) {
        let refused = draws_refused();
        // SAFETY: plain GL calls; QUAD is a static.
        unsafe {
            glViewport(0, 0, 64, 64);
            glUseProgram(p);
            glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        }
        assert_eq!(error(), GL_NO_ERROR, "draw");
        assert_eq!(draws_refused(), refused, "the draw is not refused");
        let state = wide128_draw_state(p).expect("the draw asked the program's sensitivity");
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        assert_eq!(error(), GL_NO_ERROR, "glFinish");
        state
    }

    fn open(format: u8) -> bool {
        sampled_texfmts() & gles_texfmt_bit(format) != 0
    }

    /// An RGBA8UI texture on the usampler2D: one plane, no key, the
    /// program's own shaders -- though the program is sensitive (its
    /// fragment stage samples binding 0).
    #[test]
    fn an_rgba8ui_texture_keeps_the_programs_shaders() {
        on_gc7000(|| {
            let p = usampler_program();
            assert_eq!(uint_texture(GL_RGBA8UI), GL_NO_ERROR, "RGBA8UI is sampled");
            let (sense, key) = draw(p);
            assert_eq!(sense, [0, 1], "the fragment stage samples binding 0");
            assert_eq!(key, [Wide128Planes::NONE; 2], "one plane: no key");
        });
    }

    /// An RGBA32UI texture on the same usampler2D: two planes, the fragment
    /// stage keyed on binding 0 with its B,A plane on slot 1 (the stage's
    /// one slot, then its companion), the variant drawn. Closed (before
    /// wide128-flip): the GL core stores no RGBA32UI texture, and the draw
    /// keeps the program's shaders.
    #[test]
    fn an_rgba32ui_texture_selects_the_plane_joined_variant() {
        on_gc7000(|| {
            let p = usampler_program();
            if !open(GLES_TEXFMT_RGBA32UI) {
                eprintln!(
                    "RGBA32UI is not in GC_SAMPLED_TEXFMTS (wide128-flip): checking the closed state; the open \
                     case runs in gl::draw::tests::an_rgba32ui_texture_draws_the_plane_joined_variant"
                );
                assert_ne!(uint_texture(GL_RGBA32UI), GL_NO_ERROR, "no class stores RGBA32UI");
                let (sense, key) = draw(p);
                assert_eq!(sense, [0, 1]);
                assert_eq!(key, [Wide128Planes::NONE; 2], "nothing bound is two planes");
                return;
            }
            assert_eq!(uint_texture(GL_RGBA32UI), GL_NO_ERROR, "RGBA32UI is sampled");
            let (sense, key) = draw(p);
            assert_eq!(sense, [0, 1]);
            assert_eq!(key, [Wide128Planes::NONE, Wide128Planes::NONE.with(0, 1)], "binding 0, B,A on slot 1");
            /* back to RGBA8UI on the same program: no key */
            assert_eq!(uint_texture(GL_RGBA8UI), GL_NO_ERROR);
            assert_eq!(draw(p).1, [Wide128Planes::NONE; 2]);
        });
    }

    /* ---- wrap R on a 3D RGBA32F binding ----------------------------------
     *
     * SAMP_CTRL0's W field is GL_TEXTURE_WRAP_R on a 3D texture, its U and V
     * fields S and T; the companion slot of a two-plane binding is written
     * with the same SAMP_CTRL0 as its binding's. The GL core stores
     * GL_RGBA32F in the 128-bit class on this back end (`sized_format`,
     * src/gl/texture.rs). */

    const FS_3D: &str = concat!(
        "#version 300 es\nprecision highp float;\nuniform highp sampler3D s;\nin vec2 uv;\n",
        "out vec4 o;\nvoid main() { o = texture(s, vec3(uv, 0.5)); }\n"
    );

    /// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
    fn writes(words: &[u32]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            if w & 0xf800_0000 == GC_OP_LOAD_STATE {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = words.get(i + 1 + k) {
                        out.push((base + 4 * k as u32, *v));
                    }
                }
                i += 1 + count;
                continue;
            }
            i += match w & 0xf800_0000 {
                GC_OP_DRAW_INSTANCED => 4,
                GC_OP_STALL => 2,
                _ => 1,
            };
        }
        out
    }

    fn program_3d() -> GLuint {
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS_3D));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            p
        }
    }

    /// A 2x2x2 RGBA32F 3D texture on unit 0, NEAREST, with the three wrap
    /// modes given; the error of its glTexImage3D and the class the GL core
    /// stored it in.
    fn texture_3d(s: GLenum, t: GLenum, r: GLenum) -> (GLenum, u8) {
        let texels = [1.0f32; 32];
        // SAFETY: plain GL calls; `texels` holds 2x2x2 RGBA32F texels.
        unsafe {
            let mut tex: GLuint = 0;
            glActiveTexture(GL_TEXTURE0);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_3D, tex);
            for (name, v) in [
                (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
                (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
                (GL_TEXTURE_WRAP_S, s),
                (GL_TEXTURE_WRAP_T, t),
                (GL_TEXTURE_WRAP_R, r),
            ] {
                glTexParameteri(GL_TEXTURE_3D, name, v as GLint);
            }
            glTexImage3D(
                GL_TEXTURE_3D,
                0,
                GL_RGBA32F as GLint,
                2,
                2,
                2,
                0,
                GL_RGBA,
                GL_FLOAT,
                texels.as_ptr() as *const c_void,
            );
            let c = yakogl::gl::context::current().expect("context");
            (error(), c.objs().textures[tex as usize].texfmt)
        }
    }

    /// One quad on unit 0 with `p`: the register writes of the last kick.
    fn draw_stream(p: GLuint) -> Vec<(u32, u32)> {
        let kicks0 = with_platform(|p| p.kicks.len());
        // SAFETY: plain GL calls; QUAD is a static.
        unsafe {
            glViewport(0, 0, 64, 64);
            glUseProgram(p);
            glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            glFinish();
        }
        assert_eq!(error(), GL_NO_ERROR, "draw");
        let stream = with_platform(|p| {
            assert!(p.kicks.len() > kicks0, "the quad produced no kick");
            p.kicks.last().cloned().unwrap()
        });
        writes(&stream)
    }

    /// The last value written to `reg`.
    fn last(stream: &[(u32, u32)], reg: u32) -> Option<u32> {
        stream.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
    }

    /// A 3D RGBA32F binding with wrap R unlike S and T: the slot's W field is
    /// the R mode and its U, V fields are S and T; where the binding is two
    /// planes the companion's SAMP_CTRL0 is the slot's, W included.
    #[test]
    fn wrap_r_reaches_the_slot_and_its_companion_on_a_3d_binding() {
        on_gc7000(|| {
            let p = program_3d();
            let modes = [
                (GL_CLAMP_TO_EDGE, GL_MIRRORED_REPEAT, GL_REPEAT),
                (GL_REPEAT, GL_CLAMP_TO_EDGE, GL_MIRRORED_REPEAT),
                (GL_MIRRORED_REPEAT, GL_REPEAT, GL_CLAMP_TO_EDGE),
            ];
            let hw = |m: GLenum| match m {
                GL_CLAMP_TO_EDGE => GC_TEX_WRAP_CLAMP_TO_EDGE,
                GL_MIRRORED_REPEAT => GC_TEX_WRAP_MIRROR_REPEAT,
                _ => GC_TEX_WRAP_REPEAT,
            };
            for (s, t, r) in modes {
                let (err, texfmt) = texture_3d(s, t, r);
                assert_eq!(err, GL_NO_ERROR, "glTexImage3D");
                assert!(is_wide128(texfmt) && open(texfmt), "GL_RGBA32F stored in class {texfmt}, not two planes");
                let stream = draw_stream(p);
                let ctrl0 = last(&stream, nte_descriptor_samp_ctrl0(0)).expect("the unit's sampler is written");
                let field = |k: u32| (ctrl0 >> (3 * k)) & 7;
                assert_eq!([field(0), field(1), field(2)], [hw(s), hw(t), hw(r)], "S 0x{s:x} T 0x{t:x} R 0x{r:x}");
                let companion = last(&stream, nte_descriptor_samp_ctrl0(1));
                assert_eq!(companion, Some(ctrl0), "the companion slot takes the slot's sampler state");
            }
        });
    }

    /* ---- section 5.1 of docs/notes/gc-128bit-swizzle.md, through the GL ----
     *
     * With RGBA32F/UI/I in GC_SAMPLED_TEXFMTS (8f019dae) the GL core stores
     * RGBA32UI/I and RGB32F/I/UI in the 128-bit classes on this back end, and
     * a draw that samples one is keyed and drawn as two G32R32 planes. Which
     * binding is two planes is read from the class the core stored;
     * GL_RGBA32F is stored in the 128-bit class too (`sized_format`,
     * src/gl/texture.rs). */

    /// A program whose fragment stage writes `o = <expr>`, `s` a `<sampler>`
    /// the expression reads.
    fn program_fs(sampler: &str, expr: &str) -> GLuint {
        let fs = format!(
            "#version 300 es\nprecision highp float;\nuniform highp {sampler} s;\nin vec2 uv;\n{}",
            format_args!("out vec4 o;\nvoid main() {{ o = {expr}; }}\n")
        );
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {fs}");
            p
        }
    }

    /// One 32-bit-per-channel format and the sampler and expression that read it.
    struct Fmt {
        name: &'static str,
        internal: GLenum,
        format: GLenum,
        type_: GLenum,
        sampler: &'static str,
        expr: &'static str,
        /// The class it takes where the mask has the 128-bit classes.
        class: u8,
        /// The texture has an alpha channel (RGBA, not RGB).
        alpha: bool,
        /// Alpha 1 as a stored word (1.0f or integer 1).
        one: u32,
    }

    const RGBA32UI: Fmt = Fmt {
        name: "GL_RGBA32UI",
        internal: GL_RGBA32UI,
        format: GL_RGBA_INTEGER,
        type_: GL_UNSIGNED_INT,
        sampler: "usampler2D",
        expr: "vec4(texture(s, uv)) / 255.0",
        class: GLES_TEXFMT_RGBA32UI,
        alpha: true,
        one: 1,
    };
    const RGBA32I: Fmt = Fmt {
        name: "GL_RGBA32I",
        internal: GL_RGBA32I,
        format: GL_RGBA_INTEGER,
        type_: GL_INT,
        sampler: "isampler2D",
        expr: "vec4(texture(s, uv)) / 255.0",
        class: GLES_TEXFMT_RGBA32I,
        alpha: true,
        one: 1,
    };
    const RGBA32F: Fmt = Fmt {
        name: "GL_RGBA32F",
        internal: GL_RGBA32F,
        format: GL_RGBA,
        type_: GL_FLOAT,
        sampler: "sampler2D",
        expr: "texture(s, uv)",
        class: GLES_TEXFMT_RGBA32F,
        alpha: true,
        one: 0x3f80_0000,
    };
    const RGB32F: Fmt = Fmt {
        name: "GL_RGB32F",
        internal: GL_RGB32F,
        format: GL_RGB,
        type_: GL_FLOAT,
        sampler: "sampler2D",
        expr: "texture(s, uv)",
        class: GLES_TEXFMT_RGBA32F,
        alpha: false,
        one: 0x3f80_0000,
    };
    const RGB32I: Fmt = Fmt {
        name: "GL_RGB32I",
        internal: GL_RGB32I,
        format: GL_RGB_INTEGER,
        type_: GL_INT,
        sampler: "isampler2D",
        expr: "vec4(texture(s, uv)) / 255.0",
        class: GLES_TEXFMT_RGBA32I,
        alpha: false,
        one: 1,
    };
    const RGB32UI: Fmt = Fmt {
        name: "GL_RGB32UI",
        internal: GL_RGB32UI,
        format: GL_RGB_INTEGER,
        type_: GL_UNSIGNED_INT,
        sampler: "usampler2D",
        expr: "vec4(texture(s, uv)) / 255.0",
        class: GLES_TEXFMT_RGBA32UI,
        alpha: false,
        one: 1,
    };

    /// The texel words of `f` at (x, y): distinct per texel and channel,
    /// floats exact as halves and as full words.
    fn texel(f: &Fmt, x: u32, y: u32) -> Vec<u32> {
        let n = if f.alpha { 4 } else { 3 };
        (0..n)
            .map(|c| {
                let v = 1 + c + 4 * x + 32 * y;
                if f.type_ == GL_FLOAT {
                    (v as f32).to_bits()
                } else {
                    0x1000_0000 * (c + 1) + v
                }
            })
            .collect()
    }

    /// A `w` x `h` texture of `f` on unit 0, NEAREST, every texel
    /// [`texel`]; its name and the error of its glTexImage2D.
    fn texture_2d(f: &Fmt, w: u32, h: u32) -> (GLuint, GLenum) {
        let texels: Vec<u32> = (0..h).flat_map(|y| (0..w).flat_map(move |x| texel(f, x, y))).collect();
        // SAFETY: plain GL calls; `texels` holds w x h texels of `f`.
        unsafe {
            let mut t: GLuint = 0;
            glActiveTexture(GL_TEXTURE0);
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                f.internal as GLint,
                w as GLsizei,
                h as GLsizei,
                0,
                f.format,
                f.type_,
                texels.as_ptr() as *const c_void,
            );
            (t, error())
        }
    }

    /// The class the core stored texture `name` in, and texel (0, 0) of
    /// level 0 as 32-bit words (a half of RGBA16F widened to its float's bits).
    fn stored(name: GLuint) -> (u8, Vec<u32>) {
        let c = yakogl::gl::context::current().expect("context");
        let t = &c.objs().textures[name as usize];
        assert!(!t.mem.cpu.is_null(), "texture {name} has storage with a CPU view");
        let bpp = t.bpp as usize;
        // SAFETY: texel (0, 0) of level 0 lies inside the storage the core laid out.
        let b = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(t.level_off[0] as usize), bpp) };
        let words = if t.texfmt == GLES_TEXFMT_RGBA16F {
            b.chunks(2)
                .map(|h| yakogl::gl::es3_formats::half_to_f32(u16::from_ne_bytes([h[0], h[1]])).to_bits())
                .collect()
        } else {
            b.chunks(4).map(|w| u32::from_ne_bytes([w[0], w[1], w[2], w[3]])).collect()
        };
        (t.texfmt, words)
    }

    /// Does the draw sample a texture stored in `texfmt` as two planes?
    fn planes(texfmt: u8) -> bool {
        is_wide128(texfmt) && open(texfmt)
    }

    /// What one quad on unit 0 did: the GL error after the draw call, the
    /// draws refused and the kicks submitted meanwhile, the sensitivity and
    /// key the draw worked out and the last kick's register writes.
    struct Quad {
        error: GLenum,
        refused: u32,
        kicks: usize,
        state: Option<([u32; 2], [Wide128Planes; 2])>,
        stream: Vec<(u32, u32)>,
    }

    fn quad(p: GLuint) -> Quad {
        let (k0, r0) = (with_platform(|p| p.kicks.len()), draws_refused());
        // SAFETY: plain GL calls; QUAD is a static.
        let err = unsafe {
            glViewport(0, 0, 64, 64);
            glUseProgram(p);
            glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            glGetError()
        };
        let state = wide128_draw_state(p);
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        assert_eq!(error(), GL_NO_ERROR, "glFinish");
        let (kicks, stream) = with_platform(|p| (p.kicks.len() - k0, p.kicks.last().cloned().unwrap_or_default()));
        Quad { error: err, refused: draws_refused().wrapping_sub(r0), kicks, state, stream: writes(&stream) }
    }

    /// The quad was drawn: no error, not refused, a kick.
    fn drawn(what: &str, q: &Quad) {
        assert_eq!(q.error, GL_NO_ERROR, "{what}: GL error 0x{:x}", q.error);
        assert_eq!(q.refused, 0, "{what}: refused");
        assert!(q.kicks >= 1, "{what}: no kick");
    }

    /// The quad was refused: no GL error, counted once, nothing submitted.
    fn refused(what: &str, q: &Quad) {
        assert_eq!(q.error, GL_NO_ERROR, "{what}: a refused draw is no GL error (0x{:x})", q.error);
        assert_eq!(q.refused, 1, "{what}: counted once");
        assert_eq!(q.kicks, 0, "{what}: no draw reaches the stream");
    }

    /// `n` words of the stand-in's memory at GPU address `gpu`.
    fn words_at(gpu: u64, n: usize) -> Vec<u32> {
        with_platform(|p| {
            let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
            let off = (gpu - base) as usize;
            assert!(off + n * 4 <= a.len() * 8, "0x{gpu:x}+{} runs off the allocation at 0x{base:x}", n * 4);
            // SAFETY: inside the allocation (checked just above).
            unsafe { core::slice::from_raw_parts((a.as_ptr() as *const u8).add(off) as *const u32, n) }.to_vec()
        })
    }

    /// The words of slot `slot`'s descriptor in the stream, if it wrote one.
    fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
        let a = last(stream, nte_descriptor_addr(slot))?;
        (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
    }

    fn align4(v: u32) -> u32 {
        v.next_multiple_of(4)
    }

    /// A drawn quad of a `w` x `h` texture stored as two planes: the
    /// fragment stage keyed on binding 0 with its B,A plane on slot 1, and
    /// slots 0 and 1 the descriptor pair: both the G32R32 plane format, the
    /// companion a copy but for the level addresses, its level 0 one plane
    /// (`align4(w) * align4(h)` texels of 8 bytes) after slot 0's. Returns
    /// the pair.
    fn check_pair(what: &str, class: u8, (w, h): (u32, u32), q: &Quad) -> (Vec<u32>, Vec<u32>) {
        let (sense, key) = q.state.unwrap_or_else(|| panic!("{what}: the draw asked the sensitivity"));
        assert_eq!(sense, [0, 1], "{what}: the fragment stage samples binding 0");
        assert_eq!(key[0], Wide128Planes::NONE, "{what}: the vertex stage samples nothing");
        assert_eq!(key[1].mask, 1, "{what}: binding 0 alone is keyed");
        assert_eq!(key[1].companion_of(0), Some(1), "{what}: B,A on slot 1");
        let d0 = descriptor(&q.stream, 0).unwrap_or_else(|| panic!("{what}: no descriptor in slot 0"));
        let d1 = descriptor(&q.stream, 1).unwrap_or_else(|| panic!("{what}: no companion descriptor in slot 1"));
        let ext = wide128_plane_format(class).expect("a plane format").format_ext;
        for (p, d) in [&d0, &d1].into_iter().enumerate() {
            let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
            assert_eq!(c1 & texdesc_format_ext(!0), texdesc_format_ext(ext), "{what}: plane {p}: G32R32");
        }
        for i in 16..d0.len() {
            assert_eq!(d0[i], d1[i], "{what}: descriptor word {i}: the companion is a copy");
        }
        let (a0, a1) = (d0[texdesc_lod_addr(0) as usize / 4], d1[texdesc_lod_addr(0) as usize / 4]);
        assert_eq!(a1, a0 + align4(w) * align4(h) * 8, "{what}: plane 1 one plane of 8-byte texels after plane 0");
        (d0, d1)
    }

    /// The flip pins: GC_SAMPLED_TEXFMTS holds the three 128-bit classes,
    /// and a GL_RGBA32UI, GL_RGBA32I and GL_RGB32F texture drawn through a
    /// sampling shader gets the descriptor pair -- own slot and companion,
    /// both G32R32, the planes 8 bytes a texel apart. GL_RGBA32F the same:
    /// the core stores it in the 128-bit class on this back end.
    #[test]
    fn the_flip_samples_the_128_bit_classes_as_two_planes() {
        for class in [GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA32UI, GLES_TEXFMT_RGBA32I] {
            assert_ne!(GC_SAMPLED_TEXFMTS & gles_texfmt_bit(class), 0, "class {class} in GC_SAMPLED_TEXFMTS");
            assert!(open(class), "class {class} in the mask the GL core reads");
        }
        for f in [&RGBA32UI, &RGBA32I, &RGB32F, &RGBA32F] {
            on_gc7000(|| {
                let p = program_fs(f.sampler, f.expr);
                let (w, h) = (5, 3);
                let (name, err) = texture_2d(f, w, h);
                assert_eq!(err, GL_NO_ERROR, "{}: stored", f.name);
                let (class, _) = stored(name);
                let q = quad(p);
                drawn(f.name, &q);
                assert!(planes(class), "{}: held in class {class}, not two planes", f.name);
                assert_eq!(class, f.class, "{}: the 128-bit class", f.name);
                check_pair(f.name, class, (w, h), &q);
            });
        }
    }

    /// The text `note_draw_refused` logged for the process's first refusal.
    fn first_refusal_text() -> String {
        let lines = LOG.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let l = lines
            .iter()
            .find(|l| l.contains("a draw was refused: "))
            .unwrap_or_else(|| panic!("no refusal logged: {lines:?}"));
        let rest = l.split("a draw was refused: ").nth(1).unwrap();
        rest.split(" (key 0x").next().unwrap().to_string()
    }

    /// `note_draw_refused` logs the text of a process's first refusal only,
    /// so a case that reads it runs alone in a child process of this binary
    /// (as tests/es3_wide128_refusals_gc.rs): true in the child (run the
    /// body); in the parent this runs the child, checks it passed, and
    /// returns false.
    fn in_child(name: &str) -> bool {
        const VAR: &str = "YAKOGL_W128_GC_CHILD";
        if std::env::var(VAR).as_deref() == Ok(name) {
            return true;
        }
        let out = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", name, "--test-threads=1", "--nocapture"])
            .env(VAR, name)
            .output()
            .expect("the child runs");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "{name} in a fresh process failed:\n{text}");
        assert!(text.contains("1 passed"), "{name} did not run in the child:\n{text}");
        false
    }

    /// Set the four GL_TEXTURE_SWIZZLE_* parameters of unit 0's 2D texture.
    fn swizzle(s: [GLenum; 4]) {
        // SAFETY: plain GL calls on the bound texture.
        unsafe {
            for (name, v) in [GL_TEXTURE_SWIZZLE_R, GL_TEXTURE_SWIZZLE_G, GL_TEXTURE_SWIZZLE_B, GL_TEXTURE_SWIZZLE_A]
                .into_iter()
                .zip(s)
            {
                glTexParameteri(GL_TEXTURE_2D, name, v as GLint);
            }
        }
        assert_eq!(error(), GL_NO_ERROR, "glTexParameteri swizzle {s:x?}");
    }

    const IDENTITY: [GLenum; 4] = [GL_RED, GL_GREEN, GL_BLUE, GL_ALPHA];

    /// The interim guard (b6d7a4d1, `wide128_select`): a two-plane binding
    /// whose GL texture swizzle moves a channel across the planes, swaps a
    /// plane's pair, moves alpha or puts ZERO/ONE on a lane is refused by
    /// name -- no kick, GL_NO_ERROR, the text 'texture swizzle is not
    /// joined' -- and the identity set back explicitly draws. Runs alone in a
    /// child process (the text is logged for the first refusal only).
    #[test]
    fn a_swizzle_the_planes_cannot_join_is_refused_by_name() {
        if !in_child("draw::a_swizzle_the_planes_cannot_join_is_refused_by_name") {
            return;
        }
        LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
        on_gc7000(|| {
            for f in [&RGBA32UI, &RGBA32I] {
                let p = program_fs(f.sampler, f.expr);
                let (name, err) = texture_2d(f, 2, 2);
                assert_eq!(err, GL_NO_ERROR, "{}", f.name);
                assert!(planes(stored(name).0), "{}: two planes", f.name);
                let q = quad(p);
                drawn(&format!("{}: the default swizzle", f.name), &q);
                check_pair(f.name, f.class, (2, 2), &q);
                for s in [
                    [GL_BLUE, GL_GREEN, GL_RED, GL_ALPHA],
                    [GL_GREEN, GL_RED, GL_BLUE, GL_ALPHA],
                    [GL_RED, GL_GREEN, GL_BLUE, GL_ONE],
                    [GL_ZERO; 4],
                    [GL_RED, GL_ZERO, GL_BLUE, GL_ALPHA],
                    [GL_ONE, GL_GREEN, GL_BLUE, GL_ALPHA],
                    [GL_RED, GL_GREEN, GL_BLUE, GL_RED],
                    [GL_RED, GL_GREEN, GL_ALPHA, GL_BLUE],
                ] {
                    swizzle(s);
                    let what = format!("{}: swizzle {s:x?}", f.name);
                    refused(&what, &quad(p));
                    assert_eq!(error(), GL_NO_ERROR, "{what}");
                    let why = first_refusal_text();
                    assert!(why.contains("texture swizzle is not joined"), "{what}: {why}");
                }
                /* the explicit (RED, GREEN, BLUE, ALPHA) is the identity */
                swizzle(IDENTITY);
                let q = quad(p);
                drawn(&format!("{}: the explicit identity", f.name), &q);
                check_pair(f.name, f.class, (2, 2), &q);
            }
            /* a swizzle on a one-plane texture on the same kind of program is no refusal */
            let p = program_fs(RGBA32UI.sampler, RGBA32UI.expr);
            assert_eq!(uint_texture(GL_RGBA8UI), GL_NO_ERROR);
            swizzle([GL_BLUE, GL_GREEN, GL_RED, GL_ZERO]);
            let q = quad(p);
            drawn("GL_RGBA8UI with a swizzle", &q);
            assert_eq!(q.state.map(|s| s.1), Some([Wide128Planes::NONE; 2]), "one plane, no key");
        });
    }

    /// no_alpha: GL_RGB32F, GL_RGB32I and GL_RGB32UI are stored with alpha 1
    /// (1.0f or integer 1) and drawn as two planes keyed like RGBA32 (the
    /// guard takes lanes [0,1,2,5], alpha the constant one, as the identity
    /// of a texture with no alpha), both descriptors A = ONE; a swizzle that
    /// leaves alpha alone -- explicit (RED, GREEN, BLUE, ALPHA), or (RED,
    /// GREEN, BLUE, ONE) -- is not refused, alpha ZERO is. Whether the
    /// draw's key carries `no_alpha` is said on stderr: `.w` reads 1 either
    /// way, the join's constant or the stored alpha.
    #[test]
    fn rgb32_bindings_draw_with_alpha_one() {
        for f in [&RGB32F, &RGB32I, &RGB32UI] {
            on_gc7000(|| {
                let p = program_fs(f.sampler, f.expr);
                let (w, h) = (3, 2);
                let (name, err) = texture_2d(f, w, h);
                assert_eq!(err, GL_NO_ERROR, "{}: stored", f.name);
                let (class, texel0) = stored(name);
                assert_eq!(class, f.class, "{}: the 128-bit class", f.name);
                assert_eq!(texel0[..3], texel(f, 0, 0)[..], "{}: the colour as given", f.name);
                assert_eq!(texel0.get(3), Some(&f.one), "{}: the missing alpha stored as 1", f.name);
                for s in [None, Some(IDENTITY), Some([GL_RED, GL_GREEN, GL_BLUE, GL_ONE])] {
                    if let Some(s) = s {
                        swizzle(s);
                    }
                    let what = format!("{}: swizzle {s:x?}", f.name);
                    let q = quad(p);
                    drawn(&what, &q);
                    let (d0, d1) = check_pair(&what, class, (w, h), &q);
                    for d in [&d0, &d1] {
                        let c1 = d[GC_TEXDESC_CONFIG1 as usize / 4];
                        assert_eq!(c1 & texdesc_swizzle_a(7), texdesc_swizzle_a(GC_SWIZZLE_1), "{what}: A = ONE");
                    }
                    if q.state.unwrap().1[1].lacks_alpha(0) {
                        eprintln!("{what}: the key carries no_alpha: the join writes 0x{:x} to w", f.one);
                    } else {
                        eprintln!(
                            "{what}: the key does not carry no_alpha (the GL core does not set it): w is plane 1's \
                             stored alpha 0x{:x}",
                            f.one
                        );
                    }
                }
                swizzle([GL_RED, GL_GREEN, GL_BLUE, GL_ZERO]);
                refused(&format!("{}: alpha ZERO", f.name), &quad(p));
            });
        }
    }

    /// Both planes' descriptor words of a 6x5 2D RGBA32I binding with S
    /// clamped and T mirrored: CONFIG0 2D, TILED, no CONFIG0 format; CONFIG1
    /// EXT_G32R32I with the identity swizzle; CONFIG2 no sign bit; SIZE 6x5;
    /// a row of 8 G32R32 texels; one plane per slice; one level, depth 1;
    /// plane 1's level 0 one plane after plane 0's, each plane holding its
    /// two words of every texel in the 4x4-tiled layout; and both slots'
    /// SAMP_CTRL0/1.
    #[test]
    fn both_planes_descriptor_words_of_a_2d_binding() {
        on_gc7000(|| {
            let f = &RGBA32I;
            let p = program_fs(f.sampler, f.expr);
            let (w, h) = (6, 5);
            let (name, err) = texture_2d(f, w, h);
            assert_eq!(err, GL_NO_ERROR);
            // SAFETY: plain GL calls on the bound texture.
            unsafe {
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT as GLint);
            }
            assert_eq!(stored(name).0, GLES_TEXFMT_RGBA32I);
            let q = quad(p);
            drawn(f.name, &q);
            let (d0, d1) = check_pair(f.name, f.class, (w, h), &q);
            let plane = align4(w) * align4(h) * 8;
            for (k, d) in [&d0, &d1].into_iter().enumerate() {
                let word = |r: u32| d[r as usize / 4];
                let c0 = word(GC_TEXDESC_CONFIG0);
                assert_eq!(c0 & texdesc_type(!0), texdesc_type(GC_TEX_TYPE_2D), "plane {k}: 2D");
                assert_eq!(c0 & texdesc_format(!0), 0, "plane {k}: no CONFIG0 format");
                assert_eq!(
                    c0 & texdesc_addressing_mode(!0),
                    texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED),
                    "plane {k}: TILED"
                );
                assert_eq!(
                    word(GC_TEXDESC_CONFIG1)
                        & (texdesc_format_ext(!0)
                            | texdesc_swizzle_r(7)
                            | texdesc_swizzle_g(7)
                            | texdesc_swizzle_b(7)
                            | texdesc_swizzle_a(7)),
                    texdesc_format_ext(GC_TEX_FORMAT_EXT_G32R32I)
                        | texdesc_swizzle_r(GC_SWIZZLE_R)
                        | texdesc_swizzle_g(GC_SWIZZLE_G)
                        | texdesc_swizzle_b(GC_SWIZZLE_B)
                        | texdesc_swizzle_a(GC_SWIZZLE_A),
                    "plane {k}: CONFIG1"
                );
                assert_eq!(word(GC_TEXDESC_CONFIG2), GC_TEXDESC_CONFIG2_BASE, "plane {k}: no sign bit");
                assert_eq!(word(GC_TEXDESC_SIZE), texdesc_size_width(w) | texdesc_size_height(h), "plane {k}");
                assert_eq!(word(GC_TEXDESC_LINEAR_STRIDE), align4(w) * 8, "plane {k}: a row of G32R32");
                assert_eq!(word(GC_TEXDESC_SLICE), plane, "plane {k}: one plane's layer");
                assert_eq!(
                    word(GC_TEXDESC_BASELOD),
                    texdesc_baselod_base(0) | texdesc_baselod_max(0),
                    "plane {k}: one level"
                );
                assert_eq!(word(GC_TEXDESC_3D_CONFIG), texdesc_3d_depth(1), "plane {k}: depth 1");
            }
            let (a0, a1) = (d0[texdesc_lod_addr(0) as usize / 4], d1[texdesc_lod_addr(0) as usize / 4]);
            let (p0, p1) = (words_at(u64::from(a0), plane as usize / 4), words_at(u64::from(a1), plane as usize / 4));
            for y in 0..h {
                for x in 0..w {
                    let tile = (y / 4) * (align4(w) / 4) + x / 4;
                    let at = (tile * 16 + (y % 4) * 4 + x % 4) as usize * 2;
                    let want = texel(f, x, y);
                    assert_eq!([p0[at], p0[at + 1]], want[..2], "R,G of ({x}, {y}) in plane 0");
                    assert_eq!([p1[at], p1[at + 1]], want[2..], "B,A of ({x}, {y}) in plane 1");
                }
            }
            let ctrl0 = samp_uwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
                | samp_vwrap(GC_TEX_WRAP_MIRROR_REPEAT)
                | samp_wwrap(GC_TEX_WRAP_CLAMP_TO_EDGE)
                | samp_min(GC_TEX_FILTER_NEAREST)
                | samp_mip(GC_TEX_FILTER_NONE)
                | samp_mag(GC_TEX_FILTER_NEAREST)
                | GC_SAMP_CTRL0_UNK21;
            for slot in [0, 1] {
                assert_eq!(last(&q.stream, nte_descriptor_samp_ctrl0(slot)), Some(ctrl0), "slot {slot}: SAMP_CTRL0");
                assert_eq!(
                    last(&q.stream, nte_descriptor_samp_ctrl1(slot)),
                    Some(GC_SAMP_CTRL1_UNK1),
                    "slot {slot}: SAMP_CTRL1"
                );
            }
        });
    }

    /// Wrap R on a 3D binding stored as two planes (RGBA32UI and RGBA32I on
    /// a `usampler3D` / `isampler3D`): both descriptors 3D, the W field of
    /// both slots' SAMP_CTRL0 the R mode, U and V the S and T modes. (The
    /// GL_RGBA32F row is
    /// [`wrap_r_reaches_the_slot_and_its_companion_on_a_3d_binding`].)
    #[test]
    fn wrap_r_reaches_both_slots_of_a_two_plane_3d_binding() {
        let hw = |m: GLenum| match m {
            GL_CLAMP_TO_EDGE => GC_TEX_WRAP_CLAMP_TO_EDGE,
            GL_MIRRORED_REPEAT => GC_TEX_WRAP_MIRROR_REPEAT,
            _ => GC_TEX_WRAP_REPEAT,
        };
        for (internal, ty, sampler) in
            [(GL_RGBA32UI, GL_UNSIGNED_INT, "usampler3D"), (GL_RGBA32I, GL_INT, "isampler3D")]
        {
            on_gc7000(|| {
                let p = program_fs(sampler, "vec4(texture(s, vec3(uv, 0.5))) / 255.0");
                for (s, t, r) in [
                    (GL_CLAMP_TO_EDGE, GL_MIRRORED_REPEAT, GL_REPEAT),
                    (GL_REPEAT, GL_CLAMP_TO_EDGE, GL_MIRRORED_REPEAT),
                    (GL_MIRRORED_REPEAT, GL_REPEAT, GL_CLAMP_TO_EDGE),
                ] {
                    let what = format!("{sampler}: S 0x{s:x} T 0x{t:x} R 0x{r:x}");
                    let texels = [7u32; 32];
                    // SAFETY: plain GL calls; `texels` holds 2x2x2 RGBA32 texels.
                    let tex = unsafe {
                        let mut tex: GLuint = 0;
                        glActiveTexture(GL_TEXTURE0);
                        glGenTextures(1, &mut tex);
                        glBindTexture(GL_TEXTURE_3D, tex);
                        for (name, v) in [
                            (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
                            (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
                            (GL_TEXTURE_WRAP_S, s),
                            (GL_TEXTURE_WRAP_T, t),
                            (GL_TEXTURE_WRAP_R, r),
                        ] {
                            glTexParameteri(GL_TEXTURE_3D, name, v as GLint);
                        }
                        glTexImage3D(
                            GL_TEXTURE_3D,
                            0,
                            internal as GLint,
                            2,
                            2,
                            2,
                            0,
                            GL_RGBA_INTEGER,
                            ty,
                            texels.as_ptr() as *const c_void,
                        );
                        tex
                    };
                    assert_eq!(error(), GL_NO_ERROR, "{what}: glTexImage3D");
                    assert!(planes(stored(tex).0), "{what}: two planes");
                    let q = quad(p);
                    drawn(&what, &q);
                    let key = q.state.unwrap().1;
                    assert_eq!(key, [Wide128Planes::NONE, Wide128Planes::NONE.with(0, 1)], "{what}: B,A on slot 1");
                    for slot in [0, 1] {
                        let d = descriptor(&q.stream, slot).unwrap_or_else(|| panic!("{what}: slot {slot}"));
                        let c0 = d[GC_TEXDESC_CONFIG0 as usize / 4];
                        assert_eq!(c0 & texdesc_type(!0), texdesc_type(GC_TEX_TYPE_3D), "{what}: slot {slot}: 3D");
                        let ctrl0 = last(&q.stream, nte_descriptor_samp_ctrl0(slot))
                            .unwrap_or_else(|| panic!("{what}: slot {slot}'s sampler is written"));
                        assert_eq!(ctrl0 & samp_wwrap(7), samp_wwrap(hw(r)), "{what}: slot {slot}: W is wrap R");
                        assert_eq!(
                            ctrl0 & (samp_uwrap(7) | samp_vwrap(7)),
                            samp_uwrap(hw(s)) | samp_vwrap(hw(t)),
                            "{what}: slot {slot}: U, V are S, T"
                        );
                    }
                }
            });
        }
    }

    /// isampler2D over RGBA32I and usampler2D over RGBA32UI, by `texture`
    /// with integer arithmetic on the result and by `texelFetch`: drawn,
    /// keyed on binding 0 with both planes bound (slot 0 and its companion
    /// slot 1, EXT_G32R32I).
    #[test]
    fn integer_samplers_draw_with_both_planes_bound() {
        for (f, expr) in [
            (&RGBA32I, "vec4(texture(s, uv) + ivec4(1)) / 255.0"),
            (&RGBA32UI, "vec4(texture(s, uv) >> 1u) / 255.0"),
            (&RGBA32I, "vec4(texelFetch(s, ivec2(uv), 0)) / 255.0"),
            (&RGBA32UI, "vec4(texelFetch(s, ivec2(uv), 0)) / 255.0"),
        ] {
            on_gc7000(|| {
                let what = format!("{} {}: {expr}", f.name, f.sampler);
                let p = program_fs(f.sampler, expr);
                let (name, err) = texture_2d(f, 4, 4);
                assert_eq!(err, GL_NO_ERROR, "{what}");
                assert!(planes(stored(name).0), "{what}: two planes");
                let q = quad(p);
                drawn(&what, &q);
                let key = q.state.unwrap().1;
                assert_eq!(key, [Wide128Planes::NONE, Wide128Planes::NONE.with(0, 1)], "{what}: B,A on slot 1");
                check_pair(&what, f.class, (4, 4), &q);
            });
        }
    }
}
