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

/* ---- the GL core's draw (item wide128-shader) ----------------------------
 *
 * The draw picks the key from the texture bound NOW (src/gl/draw.rs,
 * `wide128_select`): a usampler2D names RGBA8UI and RGBA32UI alike, and
 * only the second is two planes. These run a real draw through the GL core
 * and the Vivante back end on a stand-in OSAL (as
 * tests/es3_wide128_planes_gc.rs) and read the key the draw chose
 * (`gl::draw::wide128_draw_state`).
 *
 * RGBA32UI stays out of GC_SAMPLED_TEXFMTS until wide128-flip, and the mask
 * hook of the in-crate tests is `#[cfg(test)]`, so here the RGBA32UI case
 * finds the class closed, says so on stderr and checks the closed state:
 * the GL core stores no RGBA32UI texture, and the draw keeps the program's
 * shaders. From the flip on it checks the variant for real. The open case
 * runs today in the in-crate test
 * `gl::draw::tests::an_rgba32ui_texture_draws_the_plane_joined_variant`,
 * which opens the class through that hook. */

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

    /* the stand-in OSAL: plain memory, every kick completes at once */

    #[derive(Default)]
    struct Platform {
        allocs: BTreeMap<u64, Box<[u64]>>,
        bump: BTreeMap<u64, u64>,
        fence: i64,
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
        log: None,
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
}
