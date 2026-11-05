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
//! What it cannot see: the GPU, and the GL core's draw (which does not call
//! this entry yet).
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
