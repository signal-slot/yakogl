// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The texture swizzle and wrap R reach the back end (plan item int-rt-pvr;
//! dEQP-GLES3 `texture.swizzle.*`, 83 cases, and `texture.filtering.3d.*`,
//! 153 cases; `docs/es3/texture-host-repro.md`).
//!
//! Until this, `glTexParameteri(GL_TEXTURE_SWIZZLE_*)` and `GL_TEXTURE_WRAP_R`
//! were stored and read back (dEQP-GLES3 state_query.texture) and never left
//! the GL core: `GlesTextureBind` had no field for either, so no back end
//! could apply them, and every swizzle case whose picture the swizzle
//! changes failed on the AM62P (80 of 83; the three that pass are those
//! whose picture it does not change, `*_bgra` of the alpha and luminance
//! formats). Now the draw hands the back end `GlesTextureBind::swizzle`
//! (texture state, never a sampler object's: ES 3.0.6 3.8.2) and
//! `GlesTextureBind::wrap_r` (the bound sampler object's, else the
//! texture's, like wrap S and T).
//!
//! What this checks: the bind the back end receives, on a stand-in back end
//! that records every draw's texture bindings and draws nothing. Whether a
//! back end applies the two fields is each back end's own test
//! (`src/pvr`, `src/vivante`, `src/v3d`): none does yet, and the doc above
//! names the descriptor fields each has for it.
//!
//! The draws need the in-process compiler (an offline table refuses an
//! unknown program at link); the ES 2.0 build runs the one test that pins
//! what a `report-es2` bind carries, so that ES 2.0 cannot change here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tex_filtering
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es3_tex_filtering
//! ```

#![cfg(feature = "host-test")]
#![cfg_attr(not(feature = "shaderc-online"), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
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

/* ---- a context on the host: the smallest OSAL and a recording back end ---- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// The sampler state of one binding, as the back end received it: the
/// fields this file is about, plus what tells the bindings apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    unit: u8,
    type_: u8,
    layers: u16,
    wrap_s: u8,
    wrap_t: u8,
    wrap_r: u8,
    swizzle: [u8; 4],
    no_alpha: u8,
    incomplete: u8,
}

impl Seen {
    fn of(b: &GlesTextureBind) -> Seen {
        Seen {
            unit: b.unit,
            type_: b.type_,
            layers: b.layers,
            wrap_s: b.wrap_s,
            wrap_t: b.wrap_t,
            wrap_r: b.wrap_r,
            swizzle: b.swizzle,
            no_alpha: b.no_alpha,
            incomplete: b.incomplete,
        }
    }
}

/// The fragment-stage bindings of every draw since the last `take`, in draw
/// order; and the binds the upload hook was handed.
static DRAWS: Mutex<Vec<Vec<Seen>>> = Mutex::new(Vec::new());
static UPLOADS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    let binds = d.textures[..d.num_textures as usize].iter().map(Seen::of).collect();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(binds);
    0
}

unsafe extern "C" fn b_uploaded(
    b: *const GlesTextureBind,
    _level: u32,
    _x: u32,
    _y: u32,
    _w: u32,
    _h: u32,
    _src: *const c_void,
    _stride: u32,
) {
    // SAFETY: the core passes a live bind.
    let b = unsafe { &*b };
    UPLOADS.lock().unwrap_or_else(|e| e.into_inner()).push(Seen::of(b));
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// The PowerVR's layout answers (packed levels, a 3D texture's slices in a
/// grid, no native layers): the shape of the AM62P, where the failures were
/// measured. The fields under test do not depend on them.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-tex-filtering".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: Some(b_uploaded),
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        glActiveTexture(GL_TEXTURE0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    UPLOADS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The bindings of the draws since the last call, oldest first.
fn draws() -> Vec<Vec<Seen>> {
    core::mem::take(&mut *DRAWS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The one binding of the one draw since the last call.
fn one_draw() -> Seen {
    let d = draws();
    assert_eq!(d.len(), 1, "one draw: {d:?}");
    assert_eq!(d[0].len(), 1, "one binding: {d:?}");
    d[0][0]
}

/// A compiled shader, or its compile log.
fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source, a log buffer of
    // the length given.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 1 {
            return Ok(s);
        }
        let mut log = [0u8; 512];
        let mut len: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char);
        Err(String::from_utf8_lossy(&log[..len.max(0) as usize]).into_owned())
    }
}

/// `vs` and `fs` linked, current, attribute `p` at location 0 and sampler
/// `s` on unit 0; or the log when this GPU's code generator refuses one of
/// them (a look-up it does not select yet), when no draw can happen.
fn program(vs: &str, fs: &str) -> Result<GLuint, String> {
    let v = shader(GL_VERTEX_SHADER, vs)?;
    let f = shader(GL_FRAGMENT_SHADER, fs)?;
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0u8; 512];
            let mut len: GLsizei = 0;
            glGetProgramInfoLog(p, log.len() as GLsizei, &mut len, log.as_mut_ptr() as *mut c_char);
            return Err(String::from_utf8_lossy(&log[..len.max(0) as usize]).into_owned());
        }
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        assert_eq!(err(), GL_NO_ERROR);
        Ok(p)
    }
}

const QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// dEQP's renderQuad: one quad sampling unit 0 with the current program.
fn render_quad() {
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 16, 16);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    assert_eq!(err(), GL_NO_ERROR, "the quad");
}

/// A `w` x `h` texture of `format` (an unsized ES 2.0 one or GL_RGBA8),
/// NEAREST, bound to `target` on unit 0: a complete texture.
fn texture(target: GLenum, format: GLenum, w: u32, h: u32, depth: u32) -> GLuint {
    let comps = match format {
        GL_RGBA | GL_RGBA8 => 4,
        GL_RGB => 3,
        GL_LUMINANCE_ALPHA => 2,
        _ => 1,
    };
    let px = vec![0x80u8; (w * h * depth * comps) as usize];
    let mut name = 0;
    // SAFETY: plain GL calls; `px` holds the texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
        let ifmt = format as GLint;
        let fmt = if format == GL_RGBA8 { GL_RGBA } else { format };
        if target == GL_TEXTURE_3D {
            glTexImage3D(
                target,
                0,
                ifmt,
                w as GLsizei,
                h as GLsizei,
                depth as GLsizei,
                0,
                fmt,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        } else {
            glTexImage2D(
                target,
                0,
                ifmt,
                w as GLsizei,
                h as GLsizei,
                0,
                fmt,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "texture {format:#x} {w}x{h}x{depth}");
    name
}

fn delete_texture(name: GLuint) {
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

fn tex_parameter(target: GLenum, pname: GLenum, v: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(target, pname, v as GLint) };
}

/// The ES 1.00 quad of dEQP-GLES2's TextureRenderer: sampler2D on unit 0.
#[cfg(feature = "report-es2")]
const VS_100: &str = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
#[cfg(feature = "report-es2")]
const FS_100: &str =
    "precision mediump float;\nuniform sampler2D s;\nvarying vec2 v;\nvoid main() { gl_FragColor = texture2D(s, v); }\n";

/* ------------------------------------------------------------- ES 2.0 -- */

/// A `report-es2` build refuses the parameters (ES 2.0.25 table 3.10, pinned
/// in gl/texture.rs) and its bindings carry the identity swizzle and wrap R
/// REPEAT -- the zero of each field, as a zeroed `gles_texture_bind` in C --
/// so a back end that applies them changes nothing an ES 2.0 client sees.
#[cfg(all(feature = "report-es2", feature = "shaderc-online"))]
#[test]
fn es2_bindings_carry_the_identity_swizzle_and_repeat_wrap_r() {
    let _g = context();
    let t = texture(GL_TEXTURE_2D, GL_RGBA, 4, 4, 1);
    tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_BLUE);
    assert_eq!(err(), GL_INVALID_ENUM, "not an ES 2.0 parameter");
    tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
    assert_eq!(err(), GL_INVALID_ENUM, "not an ES 2.0 parameter");
    tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT);
    program(VS_100, FS_100).unwrap_or_else(|log| panic!("the ES 1.00 quad: {log}"));
    render_quad();
    let b = one_draw();
    assert_eq!((b.wrap_s, b.wrap_t), (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_MIRRORED_REPEAT));
    assert_eq!(b.wrap_r, GLES_WRAP_REPEAT, "GL's default, 0");
    assert_eq!(b.swizzle, [GLES_SWIZZLE_SAME; 4], "the identity, all zero");
    assert_eq!(GlesTextureBind::ZERO.swizzle, [0; 4]);
    assert_eq!(GlesTextureBind::ZERO.wrap_r, 0);
    delete_texture(t);
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(all(not(feature = "report-es2"), feature = "shaderc-online"))]
mod es3 {
    use super::*;

    /// The GLSL ES 3.00 quad of dEQP-GLES3's TextureRenderer (`sampler2D`),
    /// and the `sampler3D` one of texture.filtering.3d.
    const VS: &str =
        "#version 300 es\nin vec4 p;\nout vec3 v;\nvoid main() { v = p.xyz * 0.5 + 0.5; gl_Position = p; }\n";
    const FS_2D: &str = "#version 300 es\nprecision mediump float;\nuniform sampler2D s;\nin vec3 v;\nout vec4 o;\n\
                         void main() { o = texture(s, v.xy); }\n";
    const FS_3D: &str =
        "#version 300 es\nprecision mediump float;\nuniform highp sampler3D s;\nin vec3 v;\nout vec4 o;\n\
                         void main() { o = texture(s, v); }\n";

    fn quad_2d() {
        program(VS, FS_2D).unwrap_or_else(|log| panic!("the sampler2D quad: {log}"));
        render_quad();
    }

    /// The swizzle set with glTexParameteri reaches the binding as the
    /// backend contract's codes, each component "itself" where it reads its
    /// own channel: dEQP texture.swizzle's `bgra`, `one_one_red_green`,
    /// `all_zero` and the single-channel cases.
    #[test]
    fn the_swizzle_reaches_the_bind() {
        let _g = context();
        let t = texture(GL_TEXTURE_2D, GL_RGBA8, 4, 4, 1);
        quad_2d();
        assert_eq!(one_draw().swizzle, [GLES_SWIZZLE_SAME; 4], "the default, R G B A, is the identity");

        let cases: [([GLenum; 4], [u8; 4]); 5] = [
            ([GL_BLUE, GL_GREEN, GL_RED, GL_ALPHA], [GLES_SWIZZLE_BLUE, 0, GLES_SWIZZLE_RED, 0]),
            (
                [GL_ONE, GL_ONE, GL_RED, GL_GREEN],
                [GLES_SWIZZLE_ONE, GLES_SWIZZLE_ONE, GLES_SWIZZLE_RED, GLES_SWIZZLE_GREEN],
            ),
            ([GL_ZERO; 4], [GLES_SWIZZLE_ZERO; 4]),
            (
                [GL_ALPHA, GL_BLUE, GL_GREEN, GL_RED],
                [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED],
            ),
            ([GL_RED, GL_GREEN, GL_BLUE, GL_ONE], [0, 0, 0, GLES_SWIZZLE_ONE]),
        ];
        for (gl, want) in cases {
            for (k, pname) in [GL_TEXTURE_SWIZZLE_R, GL_TEXTURE_SWIZZLE_G, GL_TEXTURE_SWIZZLE_B, GL_TEXTURE_SWIZZLE_A]
                .into_iter()
                .enumerate()
            {
                tex_parameter(GL_TEXTURE_2D, pname, gl[k]);
            }
            assert_eq!(err(), GL_NO_ERROR);
            render_quad();
            assert_eq!(one_draw().swizzle, want, "swizzle {gl:x?}");
        }
        /* and back to the default */
        for pname in [GL_TEXTURE_SWIZZLE_R, GL_TEXTURE_SWIZZLE_G, GL_TEXTURE_SWIZZLE_B, GL_TEXTURE_SWIZZLE_A] {
            tex_parameter(
                GL_TEXTURE_2D,
                pname,
                [GL_RED, GL_GREEN, GL_BLUE, GL_ALPHA][(pname - GL_TEXTURE_SWIZZLE_R) as usize],
            );
        }
        render_quad();
        assert_eq!(one_draw().swizzle, [GLES_SWIZZLE_SAME; 4]);
        delete_texture(t);
    }

    /// The swizzle is in the base format's terms, so the formats of
    /// texture.swizzle.multi_channel (alpha, luminance, luminance_alpha,
    /// red, rg, rgb, rgba) carry the same codes; what their missing
    /// channels read (0, or 1 for alpha: `no_alpha`) stays the back end's
    /// format mapping to compose with.
    #[test]
    fn every_8_bit_format_carries_the_same_swizzle_codes() {
        let _g = context();
        for (format, no_alpha) in [
            (GL_ALPHA, 0u8),
            (GL_LUMINANCE, 1),
            (GL_LUMINANCE_ALPHA, 0),
            (GL_RED, 1),
            (GL_RG, 1),
            (GL_RGB, 1),
            (GL_RGBA, 0),
        ] {
            let t = texture(GL_TEXTURE_2D, format, 4, 4, 1);
            tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_ALPHA);
            tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_G, GL_BLUE);
            tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_B, GL_GREEN);
            tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_A, GL_RED);
            quad_2d();
            let b = one_draw();
            assert_eq!(
                b.swizzle,
                [GLES_SWIZZLE_ALPHA, GLES_SWIZZLE_BLUE, GLES_SWIZZLE_GREEN, GLES_SWIZZLE_RED],
                "format {format:#x}"
            );
            assert_eq!(b.no_alpha, no_alpha, "format {format:#x}: the base-format alpha rule is still the bind's");
            assert_eq!(b.incomplete, 0, "format {format:#x}");
            delete_texture(t);
        }
    }

    /// ES 3.0.6 3.8.2: a sampler object carries the filters, wraps, LOD and
    /// compare state; the swizzle is texture state and stays the texture's
    /// while a sampler object is bound to the unit.
    #[test]
    fn a_sampler_object_does_not_replace_the_swizzle() {
        use yakogl::gl::es3::*;
        let _g = context();
        let t = texture(GL_TEXTURE_2D, GL_RGBA8, 4, 4, 1);
        tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_ONE);
        tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        let mut s = 0;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenSamplers(1, &mut s);
            glSamplerParameteri(s, GL_TEXTURE_WRAP_S, GL_MIRRORED_REPEAT as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glBindSampler(0, s);
        }
        assert_eq!(err(), GL_NO_ERROR);
        quad_2d();
        let b = one_draw();
        assert_eq!(b.wrap_s, GLES_WRAP_MIRRORED_REPEAT, "the sampler object's wrap");
        assert_eq!(b.swizzle, [GLES_SWIZZLE_ONE, 0, 0, 0], "the texture's swizzle");
        // SAFETY: plain GL calls.
        unsafe {
            glBindSampler(0, 0);
            glDeleteSamplers(1, &s);
        }
        render_quad();
        let b = one_draw();
        assert_eq!(
            (b.wrap_s, b.swizzle),
            (GLES_WRAP_CLAMP_TO_EDGE, [GLES_SWIZZLE_ONE, 0, 0, 0]),
            "the texture's own again"
        );
        delete_texture(t);
    }

    /// GL_TEXTURE_WRAP_R reaches the binding like S and T: the texture's,
    /// the bound sampler object's in its place, the texture's again when
    /// the sampler object goes (texture.filtering.3d.combinations sets all
    /// three wraps on the texture; the sampler object path is 3.8.2's).
    #[test]
    fn wrap_r_reaches_the_bind_from_the_texture_or_the_sampler_object() {
        use yakogl::gl::es3::*;
        let _g = context();
        let t = texture(GL_TEXTURE_2D, GL_RGBA8, 4, 4, 1);
        quad_2d();
        assert_eq!(one_draw().wrap_r, GLES_WRAP_REPEAT, "GL's default");

        for (gl, want) in [
            (GL_MIRRORED_REPEAT, GLES_WRAP_MIRRORED_REPEAT),
            (GL_CLAMP_TO_EDGE, GLES_WRAP_CLAMP_TO_EDGE),
            (GL_REPEAT, GLES_WRAP_REPEAT),
        ] {
            tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, gl);
            assert_eq!(err(), GL_NO_ERROR);
            render_quad();
            let b = one_draw();
            assert_eq!(b.wrap_r, want, "wrap R {gl:#x}");
            assert_eq!((b.wrap_s, b.wrap_t), (GLES_WRAP_REPEAT, GLES_WRAP_REPEAT), "S and T untouched");
        }
        tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, GL_MIRRORED_REPEAT);
        let mut s = 0;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenSamplers(1, &mut s);
            glSamplerParameteri(s, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glBindSampler(0, s);
        }
        render_quad();
        assert_eq!(one_draw().wrap_r, GLES_WRAP_CLAMP_TO_EDGE, "the sampler object's wrap R");
        // SAFETY: plain GL calls.
        unsafe {
            glBindSampler(0, 0);
            glDeleteSamplers(1, &s);
        }
        render_quad();
        assert_eq!(one_draw().wrap_r, GLES_WRAP_MIRRORED_REPEAT, "the texture's again");
        delete_texture(t);
    }

    /// A 3D texture's binding (the grid of slices the PowerVR gets, as the
    /// AM62P's filtering.3d cases bind) carries wrap R and the swizzle; the
    /// draw with the texture's own 2D-array/cube/3D wraps is unchanged. A
    /// code generator that does not select `sampler3D` yet (the V3D's
    /// refusal is at the draw, not here) only skips the draw.
    #[test]
    fn a_3d_textures_binding_carries_wrap_r_and_the_swizzle() {
        let _g = context();
        let t = texture(GL_TEXTURE_3D, GL_RGBA8, 4, 8, 8);
        tex_parameter(GL_TEXTURE_3D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        tex_parameter(GL_TEXTURE_3D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        tex_parameter(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_MIRRORED_REPEAT);
        tex_parameter(GL_TEXTURE_3D, GL_TEXTURE_SWIZZLE_G, GL_ZERO);
        assert_eq!(err(), GL_NO_ERROR);
        match program(VS, FS_3D) {
            Ok(_) => {}
            Err(log) => {
                eprintln!("sampler3D is not compiled on this GPU, so the 3D binding is not drawn: {log}");
                delete_texture(t);
                return;
            }
        }
        render_quad();
        let b = one_draw();
        assert_eq!((b.type_, b.layers), (GLES_TEXTYPE_3D, 8));
        assert_eq!(
            (b.wrap_s, b.wrap_t, b.wrap_r),
            (GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_REPEAT, GLES_WRAP_MIRRORED_REPEAT)
        );
        assert_eq!(b.swizzle, [0, GLES_SWIZZLE_ZERO, 0, 0]);
        delete_texture(t);
    }

    /// The bind the upload hook hands a back end (`texture_bind_2d`, the
    /// same fields as a draw's: a back end that keeps a copy of the texels
    /// must not key on them, and sees the same values either way).
    #[test]
    fn the_upload_hooks_bind_carries_them_too() {
        let _g = context();
        let t = texture(GL_TEXTURE_2D, GL_RGBA8, 4, 4, 1);
        tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
        tex_parameter(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_B, GL_ALPHA);
        UPLOADS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let px = [0x40u8; 2 * 2 * 4];
        // SAFETY: plain GL call; `px` holds 2x2 RGBA8 texels.
        unsafe {
            glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        }
        assert_eq!(err(), GL_NO_ERROR);
        let up = UPLOADS.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(up.len(), 1, "one upload: {up:?}");
        assert_eq!(up[0].wrap_r, GLES_WRAP_CLAMP_TO_EDGE);
        assert_eq!(up[0].swizzle, [0, 0, GLES_SWIZZLE_ALPHA, 0]);
        delete_texture(t);
    }

    /// `swizzle_source`, the one table every back end composes with: the
    /// identity for SAME, then the channel or constant each code names.
    #[test]
    fn swizzle_source_is_the_identity_for_same_and_names_the_rest() {
        for k in 0..4 {
            assert_eq!(swizzle_source(k, GLES_SWIZZLE_SAME), k);
        }
        assert_eq!(swizzle_source(0, GLES_SWIZZLE_RED), 0);
        assert_eq!(swizzle_source(0, GLES_SWIZZLE_GREEN), 1);
        assert_eq!(swizzle_source(3, GLES_SWIZZLE_BLUE), 2);
        assert_eq!(swizzle_source(0, GLES_SWIZZLE_ALPHA), 3);
        assert_eq!(swizzle_source(1, GLES_SWIZZLE_ZERO), 4);
        assert_eq!(swizzle_source(2, GLES_SWIZZLE_ONE), 5);
        assert_eq!(swizzle_source(2, 7), 2, "a code outside the table reads its own channel");
    }
}
