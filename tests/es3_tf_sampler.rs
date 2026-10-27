// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Transform feedback of a vertex shader that samples textures, through the
//! GL API on the host (`src/gl/tf_capture.rs`, the evaluator's
//! `TexelSource`): the values glTransformFeedback captures, read back with
//! glMapBufferRange, against values computed here by hand from ES 3.0.6
//! sections 3.8.10-3.8.11 and GLSL ES 3.00 section 8.8.
//!
//! Every target (2D, cube, 3D, 2D array), both filters with and without
//! mipmaps, the three wrap modes, `texture`, `textureLod`, `texelFetch`,
//! `textureSize`, `textureOffset`, `textureProj`, and one format of every
//! class (normalised, signed normalised, float, packed float, shared
//! exponent, sRGB, signed and unsigned integer); then the sampler state
//! (sampler objects, the swizzle, LOD clamps, incompleteness) and what the
//! capture refuses. The texel values are exact in the storage each format
//! gets, so a value is compared bit for bit unless it says otherwise.
//!
//! The ES 2.0 report has no transform feedback: `es2_build_has_no_capture`
//! is the only test it runs. The rest needs the in-process compiler.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tf_sampler
//! ```

#![cfg(feature = "host-test")]
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
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

/// The PowerVR's texture layout questions answered as the PowerVR does
/// (packed levels, a 3D texture's slices in a grid); the unit tests in
/// `src/gl/tf_capture_tests.rs` take the GC7000's (one pitch, stacked).
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-tf-sampler".as_ptr() as *const c_char,
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
    texture_uploaded: None,
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
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        for unit in 0..4 {
            glBindSampler(unit, 0);
        }
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---------------------------------------------------------------- ES 2.0 -- */

/// The ES 2.0 report has no transform feedback at all: Begin is refused, so
/// no vertex shader is ever evaluated, sampling or not.
#[cfg(feature = "report-es2")]
#[test]
fn es2_build_has_no_capture() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glBeginTransformFeedback(GL_POINTS);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glEnable(GL_RASTERIZER_DISCARD);
        assert_eq!(err(), GL_INVALID_ENUM);
    }
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(all(not(feature = "report-es2"), feature = "shaderc-online"))]
mod es3 {
    use super::*;

    const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

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

    /// A vertex shader of `decls` and `body`: the input `a` (a vec4 a
    /// vertex) at location 0, the captured `v` of type `out_ty`.
    fn vs(decls: &str, out_ty: &str, body: &str) -> String {
        let flat = if out_ty == "vec4" { "" } else { "flat " };
        format!(
            "#version 300 es\nprecision highp float;\nprecision highp int;\nlayout(location = 0) in vec4 a;\n\
             {flat}out {out_ty} v;\n{decls}\nvoid main() {{\n v = {body};\n gl_Position = vec4(0.0);\n}}\n"
        )
    }

    /// Link `vs`, capturing `v`, current, its samplers on the units `units`
    /// names; or the log when this GPU's code generator refuses the vertex
    /// shader, or its link a sampler (a look-up the GPU does not select
    /// yet, a 3D or array texture the V3D does not sample): no draw, so no
    /// capture, can happen then.
    fn program(vs: &str, units: &[(&str, GLint)]) -> Result<GLuint, String> {
        let v = shader(GL_VERTEX_SHADER, vs)?;
        // SAFETY: plain GL calls; every pointer outlives its call.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS).expect("the fragment shader compiles"));
            let name = CString::new("v").unwrap();
            let names = [name.as_ptr()];
            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
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
            for (s, unit) in units {
                let n = CString::new(*s).unwrap();
                let loc = glGetUniformLocation(p, n.as_ptr());
                assert!(loc >= 0, "{s}");
                glUniform1i(loc, *unit);
            }
            assert_eq!(err(), GL_NO_ERROR);
            Ok(p)
        }
    }

    /// What a capturing draw came to.
    #[derive(Debug, PartialEq)]
    enum Outcome {
        /// The captured `v` of each vertex, as bits.
        Captured(Vec<[u32; 4]>),
        /// The draw's GL error; the buffer still holds its guard (`run`
        /// checks it).
        Refused(GLenum),
        /// This GPU's code generator, or the link, refused the program.
        NotCompiled,
    }

    /// Draw one point per `inputs` row with `vs` capturing, its samplers on
    /// `units`.
    fn run(vs: &str, units: &[(&str, GLint)], inputs: &[[f32; 4]]) -> Outcome {
        if let Err(log) = program(vs, units) {
            let body = vs.lines().find(|l| l.starts_with(" v = ")).unwrap_or(vs);
            eprintln!("not compiled on this GPU, so not captured:{body}\n  {log}");
            return Outcome::NotCompiled;
        }
        let n = inputs.len();
        let bytes = 16 * n;
        let guard = vec![0xcdu8; bytes];
        // SAFETY: plain GL calls; the client array and the guard outlive the
        // draw, the mapping is read inside its range.
        unsafe {
            let mut b = 0;
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, b);
            glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, bytes as GLsizeiptr, guard.as_ptr().cast(), GL_DYNAMIC_READ);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, inputs.as_ptr().cast());
            glBeginTransformFeedback(GL_POINTS);
            glDrawArrays(GL_POINTS, 0, n as GLsizei);
            let draw_err = err();
            glEndTransformFeedback();
            glDisableVertexAttribArray(0);
            assert_eq!(err(), GL_NO_ERROR);
            let p =
                glMapBufferRange(GL_TRANSFORM_FEEDBACK_BUFFER, 0, bytes as GLsizeiptr, GL_MAP_READ_BIT) as *const u8;
            assert!(!p.is_null(), "mapped");
            let got = core::slice::from_raw_parts(p, bytes).to_vec();
            glUnmapBuffer(GL_TRANSFORM_FEEDBACK_BUFFER);
            glDeleteBuffers(1, &b);
            if draw_err != GL_NO_ERROR {
                assert_eq!(got, guard, "a refused capture writes nothing");
                return Outcome::Refused(draw_err);
            }
            Outcome::Captured(
                got.chunks(16)
                    .map(|c| {
                        core::array::from_fn(|k| {
                            u32::from_ne_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]])
                        })
                    })
                    .collect(),
            )
        }
    }

    /// [`run`] of an integer `v`: the captured bits, or None when this GPU
    /// does not compile the shader. A refused capture fails the test.
    fn try_ints(vs: &str, units: &[(&str, GLint)], inputs: &[[f32; 4]]) -> Option<Vec<[u32; 4]>> {
        match run(vs, units, inputs) {
            Outcome::Captured(v) => Some(v),
            Outcome::NotCompiled => None,
            Outcome::Refused(e) => panic!("{vs}: the capture refused it (0x{e:04x})"),
        }
    }

    /// [`try_ints`] of a float `v`.
    fn try_floats(vs: &str, units: &[(&str, GLint)], inputs: &[[f32; 4]]) -> Option<Vec<[f32; 4]>> {
        try_ints(vs, units, inputs).map(|v| v.into_iter().map(|x| x.map(f32::from_bits)).collect())
    }

    /// [`try_floats`] of a shader every GPU compiles: captured.
    fn floats(vs: &str, units: &[(&str, GLint)], inputs: &[[f32; 4]]) -> Vec<[f32; 4]> {
        try_floats(vs, units, inputs).unwrap_or_else(|| panic!("{vs}: every GPU compiles it"))
    }

    /// [`try_ints`] of a shader every GPU compiles: captured.
    fn ints(vs: &str, units: &[(&str, GLint)], inputs: &[[f32; 4]]) -> Vec<[u32; 4]> {
        try_ints(vs, units, inputs).unwrap_or_else(|| panic!("{vs}: every GPU compiles it"))
    }

    /// A new texture bound to `target` on `unit`, with these filters and
    /// one wrap mode for S, T and R.
    fn texture(target: GLenum, unit: GLuint, min: GLenum, mag: GLenum, wrap: GLenum) -> GLuint {
        let mut t = 0;
        // SAFETY: plain GL calls; one name.
        unsafe {
            glGenTextures(1, &mut t);
            glActiveTexture(GL_TEXTURE0 + unit);
            glBindTexture(target, t);
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min as GLint);
            glTexParameteri(target, GL_TEXTURE_MAG_FILTER, mag as GLint);
            glTexParameteri(target, GL_TEXTURE_WRAP_S, wrap as GLint);
            glTexParameteri(target, GL_TEXTURE_WRAP_T, wrap as GLint);
            glTexParameteri(target, GL_TEXTURE_WRAP_R, wrap as GLint);
            glActiveTexture(GL_TEXTURE0);
        }
        t
    }

    /// Level `level` of the 2D (or cube face) `target` of the texture bound
    /// on `unit`: RGBA16F from floats (every value here is exact in a half).
    fn image_f(unit: GLuint, target: GLenum, level: GLint, w: GLsizei, h: GLsizei, texels: &[[f32; 4]]) {
        assert_eq!(texels.len(), (w * h) as usize);
        // SAFETY: plain GL calls; the texels outlive the upload.
        unsafe {
            glActiveTexture(GL_TEXTURE0 + unit);
            glTexImage2D(target, level, GL_RGBA16F as GLint, w, h, 0, GL_RGBA, GL_FLOAT, texels.as_ptr().cast());
            glActiveTexture(GL_TEXTURE0);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }

    /// Level `level` of the 3D or 2D array texture bound on `unit`, RGBA16F.
    fn image_3d(unit: GLuint, target: GLenum, level: GLint, w: GLsizei, h: GLsizei, d: GLsizei, texels: &[[f32; 4]]) {
        assert_eq!(texels.len(), (w * h * d) as usize);
        // SAFETY: plain GL calls; the texels outlive the upload.
        unsafe {
            glActiveTexture(GL_TEXTURE0 + unit);
            glTexImage3D(target, level, GL_RGBA16F as GLint, w, h, d, 0, GL_RGBA, GL_FLOAT, texels.as_ptr().cast());
            glActiveTexture(GL_TEXTURE0);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }

    /// The red lanes of `v`.
    fn reds(v: &[[f32; 4]]) -> Vec<f32> {
        v.iter().map(|x| x[0]).collect()
    }

    /* ---- 2D: wrap modes, filters, levels, look-up functions ---------------- */

    /// NEAREST on a 4x1 image whose texel i is red i, through the three wrap
    /// modes (ES 3.0.6 table 3.19). s = -0.375 is u = -1.5, texel -2:
    /// REPEAT 2, CLAMP_TO_EDGE 0, MIRRORED_REPEAT 1. s = 1.125 is texel 4:
    /// 0, 3, 3. s = 0.625 is texel 2 in all three.
    #[test]
    fn nearest_through_the_three_wrap_modes() {
        let _g = context();
        let row: Vec<[f32; 4]> = (0..4).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect();
        for (unit, wrap) in [(0, GL_REPEAT), (1, GL_CLAMP_TO_EDGE), (2, GL_MIRRORED_REPEAT)] {
            texture(GL_TEXTURE_2D, unit, GL_NEAREST, GL_NEAREST, wrap);
            image_f(unit, GL_TEXTURE_2D, 0, 4, 1, &row);
        }
        let src = vs(
            "uniform sampler2D r, c, m;",
            "vec4",
            "vec4(texture(r, a.xy).x, texture(c, a.xy).x, texture(m, a.xy).x, 0.0)",
        );
        let got = floats(
            &src,
            &[("r", 0), ("c", 1), ("m", 2)],
            &[[-0.375, 0.5, 0.0, 0.0], [1.125, 0.5, 0.0, 0.0], [0.625, 0.5, 0.0, 0.0]],
        );
        assert_eq!(got, [[2.0, 0.0, 1.0, 0.0], [0.0, 3.0, 3.0, 0.0], [2.0, 2.0, 2.0, 0.0]]);
    }

    /// LINEAR on a 2x2 image of reds 0 1 / 2 3 (3.8.11): the centre reads the
    /// mean 1.5; (0.25, 0.5) blends the left column, 1; (0.375, 0.25) is a
    /// quarter of the way to texel 1, 0.25; s = 0 straddles the left edge,
    /// which REPEAT blends with the right column (0.5) and CLAMP_TO_EDGE does
    /// not (0).
    #[test]
    fn linear_blends_the_four_texels_and_wraps_across_the_edge() {
        let _g = context();
        let quad: Vec<[f32; 4]> = (0..4).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect();
        texture(GL_TEXTURE_2D, 0, GL_LINEAR, GL_LINEAR, GL_CLAMP_TO_EDGE);
        image_f(0, GL_TEXTURE_2D, 0, 2, 2, &quad);
        texture(GL_TEXTURE_2D, 1, GL_LINEAR, GL_LINEAR, GL_REPEAT);
        image_f(1, GL_TEXTURE_2D, 0, 2, 2, &quad);
        let src = vs("uniform sampler2D c, r;", "vec4", "vec4(texture(c, a.xy).x, texture(r, a.xy).x, 0.0, 0.0)");
        let got = floats(
            &src,
            &[("c", 0), ("r", 1)],
            &[[0.5, 0.5, 0.0, 0.0], [0.25, 0.5, 0.0, 0.0], [0.375, 0.25, 0.0, 0.0], [0.0, 0.25, 0.0, 0.0]],
        );
        assert_eq!(got, [[1.5, 1.5, 0.0, 0.0], [1.0, 1.0, 0.0, 0.0], [0.25, 0.25, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0]]);
    }

    /// Three levels of a 4x4 image, every texel of level k red 10 (k + 1).
    fn mip_chain(unit: GLuint, min: GLenum) -> GLuint {
        let t = texture(GL_TEXTURE_2D, unit, min, GL_NEAREST, GL_CLAMP_TO_EDGE);
        for (k, w) in [4, 2, 1].into_iter().enumerate() {
            let texels = vec![[10.0 * (k + 1) as f32, 0.0, 0.0, 1.0]; (w * w) as usize];
            image_f(unit, GL_TEXTURE_2D, k as GLint, w, w, &texels);
        }
        t
    }

    /// textureLod's level selection (3.8.10.4). NEAREST_MIPMAP_NEAREST: the
    /// nearest level, ceil(lambda + 1/2) - 1, so 0.25 reads level 0, 0.75 and
    /// 1.5 level 1, 1.75 level 2, 5 the last level, and -1 magnifies level 0.
    /// LINEAR_MIPMAP_LINEAR: levels floor(lambda) and the next, blended by
    /// the fraction (0.25 is 12.5, 0.75 is 17.5, 1.75 is 27.5, 1.5 is 25),
    /// past q the last level. texture() in a vertex shader reads level of
    /// detail 0. Every lambda is a binary fraction, so the blends are exact.
    #[test]
    fn texture_lod_selects_and_blends_mip_levels() {
        let _g = context();
        mip_chain(0, GL_NEAREST_MIPMAP_NEAREST);
        mip_chain(1, GL_LINEAR_MIPMAP_LINEAR);
        let src = vs(
            "uniform sampler2D n, l;",
            "vec4",
            "vec4(textureLod(n, a.xy, a.z).x, textureLod(l, a.xy, a.z).x, texture(n, a.xy).x, texture(l, a.xy).x)",
        );
        let lods = [0.25, 0.75, 1.75, 5.0, -1.0, 1.5];
        let inputs: Vec<[f32; 4]> = lods.iter().map(|&l| [0.3, 0.6, l, 0.0]).collect();
        let got = floats(&src, &[("n", 0), ("l", 1)], &inputs);
        let want_n = [10.0, 20.0, 30.0, 30.0, 10.0, 20.0];
        let want_l = [12.5, 17.5, 27.5, 30.0, 10.0, 25.0];
        for (k, v) in got.iter().enumerate() {
            assert_eq!([v[0], v[1], v[2], v[3]], [want_n[k], want_l[k], 10.0, 10.0], "lod {}", lods[k]);
        }
    }

    /// GL_TEXTURE_MAX_LEVEL and GL_TEXTURE_MAX_LOD clamp the selection
    /// (3.8.10.4: q is min(p, max level); lambda is clamped to the LOD
    /// range): max level 1 stops lod 1.6 at level 1, max LOD 0.5 makes it
    /// a blend of levels 0 and 1, and a min LOD of 2 makes lod 0 read level
    /// 2. Lambda <= 0 after the clamp magnifies, so min LOD 2 also switches
    /// NEAREST_MIPMAP_NEAREST's lod -1 to minification.
    #[test]
    fn max_level_and_the_lod_range_clamp_the_selection() {
        let _g = context();
        let src = vs("uniform sampler2D s;", "vec4", "textureLod(s, a.xy, a.z)");
        // SAFETY: plain GL calls on the texture just bound.
        unsafe {
            mip_chain(0, GL_LINEAR_MIPMAP_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, 1);
            assert_eq!(reds(&floats(&src, &[("s", 0)], &[[0.5, 0.5, 1.6, 0.0]])), [20.0]);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, 1000);
            glTexParameterf(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 0.5);
            assert_eq!(reds(&floats(&src, &[("s", 0)], &[[0.5, 0.5, 1.6, 0.0]])), [15.0]);
            glTexParameterf(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 1000.0);
            glTexParameterf(GL_TEXTURE_2D, GL_TEXTURE_MIN_LOD, 2.0);
            assert_eq!(reds(&floats(&src, &[("s", 0)], &[[0.5, 0.5, 0.0, 0.0], [0.5, 0.5, -1.0, 0.0]])), [30.0, 30.0]);
        }
    }

    /// texelFetch reads the texel at integer coordinates of the level it
    /// names, unfiltered and unwrapped (GLSL ES 3.00 8.9.2); outside the
    /// image it is undefined and reads 0 here. texelFetchOffset and
    /// textureOffset move the texel; textureProj divides by the last lane.
    /// The image: 4x2 reds x + 4y, level 1 reds 100 and 101, level 2 red 200.
    /// A look-up a GPU's code generator does not select (the PowerVR has no
    /// offsets yet) is skipped there; the other GPUs capture it.
    #[test]
    fn texel_fetch_offsets_and_projection() {
        let _g = context();
        texture(GL_TEXTURE_2D, 0, GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        let l0: Vec<[f32; 4]> = (0..8).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect();
        image_f(0, GL_TEXTURE_2D, 0, 4, 2, &l0);
        image_f(0, GL_TEXTURE_2D, 1, 2, 1, &[[100.0, 0.0, 0.0, 1.0], [101.0, 0.0, 0.0, 1.0]]);
        image_f(0, GL_TEXTURE_2D, 2, 1, 1, &[[200.0, 0.0, 0.0, 1.0]]);
        let s = [("s", 0)];
        let inputs = [[3.0, 1.0, 0.0, 1.0], [1.0, 0.0, 1.0, 1.0], [0.0, 0.0, 2.0, 2.0], [4.0, 0.0, 0.0, 1.0]];
        let fetch = vs("uniform sampler2D s;", "vec4", "texelFetch(s, ivec2(a.xy), int(a.z))");
        if let Some(v) = try_floats(&fetch, &s, &inputs) {
            assert_eq!(reds(&v), [7.0, 101.0, 200.0, 0.0], "(4, 0) is outside: 0");
        }
        let offset = vs("uniform sampler2D s;", "vec4", "texelFetchOffset(s, ivec2(a.xy), 0, ivec2(-1, 0))");
        if let Some(v) = try_floats(&offset, &s, &inputs) {
            assert_eq!(reds(&v), [6.0, 0.0, 0.0, 3.0], "(-1, 0) from (0, 0) is outside: 0");
        }
        /* texel (0, 0) moved by (2, 1) is texel 6 */
        let offset = vs("uniform sampler2D s;", "vec4", "textureOffset(s, vec2(0.125, 0.25) * a.w, ivec2(2, 1))");
        if let Some(v) = try_floats(&offset, &s, &inputs[..1]) {
            assert_eq!(reds(&v), [6.0]);
        }
        /* (0.875, 0.75) / 0.5 is (1.75, 1.5): clamped to texel (3, 1), 7;
         * the same over 1.0 at w = 2 */
        let proj = vs("uniform sampler2D s;", "vec4", "textureProj(s, vec3(0.875 * a.w, 0.75 * a.w, 0.5 * a.w))");
        if let Some(v) = try_floats(&proj, &s, &inputs[..3]) {
            assert_eq!(reds(&v), [7.0, 7.0, 7.0]);
        }
        let proj = vs("uniform sampler2D s;", "vec4", "textureProj(s, vec4(0.25 * a.w, 0.25, 9.0, a.w))");
        if let Some(v) = try_floats(&proj, &s, &inputs[2..3]) {
            assert_eq!(reds(&v), [1.0], "the vec4 form: (0.5, 0.25) / 2 is texel (1, 0)");
        }
    }

    /// textureSize of every target, at levels 0 and 1.
    #[test]
    fn texture_size_of_each_target() {
        let _g = context();
        texture(GL_TEXTURE_2D, 0, GL_NEAREST, GL_NEAREST, GL_REPEAT);
        image_f(0, GL_TEXTURE_2D, 0, 4, 2, &[[0.0; 4]; 8]);
        texture(GL_TEXTURE_CUBE_MAP, 1, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        for f in 0..6 {
            image_f(1, GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, 2, 2, &[[0.0; 4]; 4]);
        }
        texture(GL_TEXTURE_3D, 2, GL_NEAREST, GL_NEAREST, GL_REPEAT);
        image_3d(2, GL_TEXTURE_3D, 0, 4, 2, 8, &[[0.0; 4]; 64]);
        texture(GL_TEXTURE_2D_ARRAY, 3, GL_NEAREST, GL_NEAREST, GL_REPEAT);
        image_3d(3, GL_TEXTURE_2D_ARRAY, 0, 2, 4, 3, &[[0.0; 4]; 24]);
        let src = vs(
            "uniform sampler2D s2; uniform samplerCube sc; uniform highp sampler3D s3; uniform highp sampler2DArray sa;",
            "ivec4",
            "ivec4(textureSize(s2, 0), textureSize(sc, 0))",
        );
        let units = [("s2", 0), ("sc", 1)];
        assert_eq!(ints(&src, &units, &[[0.0; 4]]), [[4, 2, 2, 2]], "2D (4, 2), cube (2, 2)");
        /* a level other than a literal 0: not selected on the GC7000 yet */
        let levels = [[0.0; 4], [1.0, 0.0, 0.0, 0.0]];
        let src = vs(
            "uniform sampler2D s2; uniform samplerCube sc;",
            "ivec4",
            "ivec4(textureSize(s2, int(a.x)), textureSize(sc, int(a.x)))",
        );
        if let Some(got) = try_ints(&src, &units, &levels) {
            assert_eq!(got, [[4, 2, 2, 2], [2, 1, 1, 1]], "and level 1");
        }
        let src = vs("uniform highp sampler3D s3;", "ivec4", "ivec4(textureSize(s3, int(a.x)), 0)");
        if let Some(got) = try_ints(&src, &[("s3", 2)], &levels) {
            assert_eq!(got, [[4, 2, 8, 0], [2, 1, 4, 0]], "3D: the depth halves too");
        }
        /* the texture() read keeps the sampler in the PowerVR's uniform
         * table: one read only by textureSize has no location there
         * (shaderc/online/powervr/chain.rs keeps the sampled rows) */
        let src = vs(
            "uniform highp sampler2DArray sa;",
            "ivec4",
            "ivec4(textureSize(sa, int(a.x)), int(texture(sa, vec3(0.0)).x))",
        );
        if let Some(got) = try_ints(&src, &[("sa", 3)], &levels) {
            assert_eq!(got, [[2, 4, 3, 0], [1, 2, 3, 0]], "2D array: the layer count stays");
        }
    }

    /* ---- cube maps ----------------------------------------------------------- */

    /// A cube map whose face f (+X, -X, +Y, -Y, +Z, -Z) is red f + 1,
    /// 2x2 a face.
    fn cube(unit: GLuint, min: GLenum, mag: GLenum) {
        texture(GL_TEXTURE_CUBE_MAP, unit, min, mag, GL_CLAMP_TO_EDGE);
        for f in 0..6u32 {
            image_f(unit, GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, 2, 2, &[[f as f32 + 1.0, 0.0, 0.0, 1.0]; 4]);
        }
    }

    /// The major axis selects the face (ES 3.0.6 table 3.21); LINEAR at the
    /// direction (1, 0, 1), on +X's edge with +Z, blends the texels across
    /// that edge (seamless filtering, 3.8.9.1): (1 + 5) / 2.
    #[test]
    fn cube_faces_and_a_seamless_edge() {
        let _g = context();
        cube(0, GL_NEAREST, GL_NEAREST);
        cube(1, GL_LINEAR, GL_LINEAR);
        let src = vs(
            "uniform samplerCube n, l;",
            "vec4",
            "vec4(texture(n, a.xyz).x, textureLod(n, a.xyz, 0.0).x, texture(l, a.xyz).x, 0.0)",
        );
        let dirs = [
            [1.0, 0.2, -0.3],
            [-1.0, 0.1, 0.4],
            [0.3, 1.0, 0.1],
            [0.2, -1.0, -0.1],
            [0.1, 0.2, 1.0],
            [-0.4, 0.3, -1.0],
        ];
        let mut inputs: Vec<[f32; 4]> = dirs.iter().map(|d| [d[0], d[1], d[2], 0.0]).collect();
        inputs.push([1.0, 0.0, 1.0, 0.0]);
        let got = floats(&src, &[("n", 0), ("l", 1)], &inputs);
        for (f, v) in got[..6].iter().enumerate() {
            let want = f as f32 + 1.0;
            assert_eq!(v[..2], [want, want], "face {f}");
            /* LINEAR inside one face: the four weights of a constant face
             * sum to 1 within a rounding */
            assert!((v[2] - want).abs() < 1e-5, "face {f}, LINEAR: {}", v[2]);
        }
        assert_eq!(got[6][2], 3.0, "seamless: half +X (1), half +Z (5)");
    }

    /* ---- 3D and 2D arrays ---------------------------------------------------- */

    /// A 2x2x2 image of reds x + 2y + 4z: NEAREST picks a texel, LINEAR at
    /// the centre is the mean 3.5, the R wrap mode wraps the third
    /// coordinate, texelFetch reads (1, 1, 1), and level 1 (1x1x1, red 100)
    /// is read through textureLod.
    #[test]
    fn three_d_nearest_linear_wrap_r_and_levels() {
        let _g = context();
        let cube8: Vec<[f32; 4]> = (0..8).map(|i| [i as f32, 0.0, 0.0, 1.0]).collect();
        texture(GL_TEXTURE_3D, 0, GL_NEAREST_MIPMAP_NEAREST, GL_NEAREST, GL_REPEAT);
        image_3d(0, GL_TEXTURE_3D, 0, 2, 2, 2, &cube8);
        image_3d(0, GL_TEXTURE_3D, 1, 1, 1, 1, &[[100.0, 0.0, 0.0, 1.0]]);
        texture(GL_TEXTURE_3D, 1, GL_LINEAR, GL_LINEAR, GL_CLAMP_TO_EDGE);
        image_3d(1, GL_TEXTURE_3D, 0, 2, 2, 2, &cube8);
        let src = vs(
            "uniform highp sampler3D r, c;",
            "vec4",
            "vec4(textureLod(r, a.xyz, a.w).x, texture(c, a.xyz).x, texelFetch(r, ivec3(1), 0).x, 0.0)",
        );
        let inputs = [[0.25, 0.75, 0.75, 0.0], [0.5, 0.5, 0.5, 0.0], [0.25, 0.25, 1.25, 0.0], [0.25, 0.25, 0.25, 1.0]];
        let Some(got) = try_floats(&src, &[("r", 0), ("c", 1)], &inputs) else { return };
        assert_eq!(got[0][0], 6.0, "texel (0, 1, 1)");
        assert_eq!(got[1][1], 3.5, "the mean of the eight");
        /* r = 1.25 is w = 2.5, slice 2: REPEAT slice 0, and CLAMP_TO_EDGE's
         * LINEAR reads slice 1 only (w - 1/2 = 2, clamped) */
        assert_eq!(got[2][0], 0.0);
        assert_eq!(got[2][1], 4.0, "texel (0, 0, 1)");
        assert_eq!(got[3][0], 100.0, "level 1");
        assert!(got.iter().all(|v| v[2] == 7.0), "texelFetch (1, 1, 1)");
    }

    /// A 2x1 array of three layers, red 10 layer + x: the layer is
    /// floor(r + 1/2) clamped (3.8.10), never filtered; texelFetch names it.
    #[test]
    fn two_d_array_layers() {
        let _g = context();
        let texels: Vec<[f32; 4]> = (0..6).map(|i| [(10 * (i / 2) + i % 2) as f32, 0.0, 0.0, 1.0]).collect();
        texture(GL_TEXTURE_2D_ARRAY, 0, GL_NEAREST, GL_LINEAR, GL_CLAMP_TO_EDGE);
        image_3d(0, GL_TEXTURE_2D_ARRAY, 0, 2, 1, 3, &texels);
        let src = vs(
            "uniform highp sampler2DArray s;",
            "vec4",
            "vec4(texture(s, a.xyz).x, texelFetch(s, ivec3(0, 0, 1), 0).x, 0.0, 0.0)",
        );
        let inputs = [[0.75, 0.5, 1.6, 0.0], [0.75, 0.5, -3.0, 0.0], [0.75, 0.5, 7.0, 0.0], [0.5, 0.5, 0.49, 0.0]];
        let Some(got) = try_floats(&src, &[("s", 0)], &inputs) else { return };
        assert_eq!(reds(&got), [21.0, 1.0, 21.0, 0.5], "layers 2, 0, 2, and LINEAR within layer 0");
        assert!(got.iter().all(|v| v[1] == 10.0));
    }

    /* ---- formats ------------------------------------------------------------- */

    /// Texel (0, 0) of a 1x1 image of `internal` from (format, type) `bytes`,
    /// through `texture(s, vec2(0.5))` of the sampler type `sampler` and a
    /// `v` of `out_ty`: the captured bits, or None when this back end
    /// stores no such format (glTexImage2D refused it; the GC7000 and the
    /// V3D sample no integer class).
    fn one_texel(
        internal: GLenum,
        format: GLenum,
        ty: GLenum,
        bytes: &[u8],
        sampler: &str,
        out_ty: &str,
    ) -> Option<[u32; 4]> {
        texture(GL_TEXTURE_2D, 0, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        // SAFETY: plain GL calls; the bytes outlive the upload.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 1, 1, 0, format, ty, bytes.as_ptr().cast()) };
        if err() != GL_NO_ERROR {
            eprintln!("0x{internal:04x}: not stored on this back end; skipped");
            return None;
        }
        let src = vs(&format!("uniform highp {sampler} s;"), out_ty, "texture(s, vec2(0.5))");
        try_ints(&src, &[("s", 0)], &[[0.0; 4]]).map(|v| v[0])
    }

    fn le_f32s(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_ne_bytes()).collect()
    }

    fn f(v: [u32; 4]) -> [f32; 4] {
        v.map(f32::from_bits)
    }

    /// One format of every class, as ES 3.0.6 3.8.14.1 has the texel read:
    /// unsigned normalised c / (2^n - 1), signed normalised max(c / 127, -1),
    /// the float and packed-float values exactly, the missing components
    /// (0, 0, 0, 1), integers as they are.
    #[test]
    fn every_format_class_reads_as_the_specification_converts_it() {
        let _g = context();
        let u = |b: u8| b as f32 / 255.0;
        let rgba8 = one_texel(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, &[255, 51, 0, 128], "sampler2D", "vec4");
        assert_eq!(f(rgba8.unwrap()), [1.0, u(51), 0.0, u(128)], "RGBA8");
        let r8 = one_texel(GL_R8, GL_RED, GL_UNSIGNED_BYTE, &[51], "sampler2D", "vec4");
        assert_eq!(f(r8.unwrap()), [u(51), 0.0, 0.0, 1.0], "R8");
        let la = one_texel(GL_LUMINANCE_ALPHA, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE, &[51, 128], "sampler2D", "vec4");
        assert_eq!(f(la.unwrap()), [u(51), u(51), u(51), u(128)], "LUMINANCE_ALPHA");
        if let Some(v) = one_texel(GL_RGBA8_SNORM, GL_RGBA, GL_BYTE, &[127, 0x81, 0x80, 0], "sampler2D", "vec4") {
            assert_eq!(f(v), [1.0, -1.0, -1.0, 0.0], "RGBA8_SNORM: -128 and -127 both read -1");
        }
        let word = 1023 | (512 << 10) | (3 << 30);
        let rgb10a2 = one_texel(
            GL_RGB10_A2,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            &u32::to_ne_bytes(word),
            "sampler2D",
            "vec4",
        );
        let v = f(rgb10a2.unwrap());
        /* stored RGB10_A2, or RGBA8 where the back end samples no 10-bit
         * class (the GC7000): two bits lost */
        assert_eq!([v[0], v[2], v[3]], [1.0, 0.0, 1.0], "RGB10_A2");
        assert!((v[1] - 512.0 / 1023.0).abs() < 2.0 / 255.0, "RGB10_A2 green {}", v[1]);
        /* the float classes the V3D samples none of are skipped there */
        if let Some(v) =
            one_texel(GL_R11F_G11F_B10F, GL_RGB, GL_FLOAT, &le_f32s(&[1.5, 0.25, 2.0]), "sampler2D", "vec4")
        {
            assert_eq!(f(v), [1.5, 0.25, 2.0, 1.0], "R11F_G11F_B10F");
        }
        if let Some(v) = one_texel(GL_RGB9_E5, GL_RGB, GL_FLOAT, &le_f32s(&[1.0, 0.5, 0.25]), "sampler2D", "vec4") {
            assert_eq!(f(v), [1.0, 0.5, 0.25, 1.0], "RGB9_E5");
        }
        if let Some(v) = one_texel(GL_RGB16F, GL_RGB, GL_FLOAT, &le_f32s(&[-2.5, 1024.0, 0.125]), "sampler2D", "vec4") {
            assert_eq!(f(v), [-2.5, 1024.0, 0.125, 1.0], "RGB16F");
        }
        let r32 = one_texel(GL_R32F, GL_RED, GL_FLOAT, &le_f32s(&[0.1]), "sampler2D", "vec4");
        assert_eq!(f(r32.unwrap()), [0.1, 0.0, 0.0, 1.0], "R32F keeps all 32 bits");
        if let Some(v) = one_texel(GL_RG32F, GL_RG, GL_FLOAT, &le_f32s(&[3.0, -0.5]), "sampler2D", "vec4") {
            assert_eq!(f(v), [3.0, -0.5, 0.0, 1.0], "RG32F");
        }
        let srgb = one_texel(GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE, &[0, 255, 188, 51], "sampler2D", "vec4");
        let v = f(srgb.unwrap());
        let lin = (((188.0 / 255.0) + 0.055) / 1.055f64).powf(2.4) as f32;
        assert_eq!([v[0], v[1], v[3]], [0.0, 1.0, u(51)], "SRGB8_ALPHA8: alpha is linear");
        assert!((v[2] - lin).abs() < 1e-6, "SRGB8_ALPHA8 blue {} against {lin}", v[2]);

        /* the integer classes: an isampler2D or usampler2D, no conversion */
        if let Some(v) = one_texel(GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE, &[0xff, 127, 0x80, 5], "isampler2D", "ivec4") {
            assert_eq!(v, [-1i32 as u32, 127, -128i32 as u32, 5], "RGBA8I");
        }
        if let Some(v) = one_texel(GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, &[200], "usampler2D", "uvec4") {
            assert_eq!(v, [200, 0, 0, 1], "R8UI: alpha is the integer 1");
        }
        let shorts: Vec<u8> = [65535u16, 1, 2, 3].iter().flat_map(|x| x.to_ne_bytes()).collect();
        if let Some(v) = one_texel(GL_RGBA16UI, GL_RGBA_INTEGER, GL_UNSIGNED_SHORT, &shorts, "usampler2D", "uvec4") {
            assert_eq!(v, [65535, 1, 2, 3], "RGBA16UI");
        }
        let words: Vec<u8> = [i32::MIN, -1, 7, i32::MAX].iter().flat_map(|x| x.to_ne_bytes()).collect();
        if let Some(v) = one_texel(GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, &words, "isampler2D", "ivec4") {
            assert_eq!(v, [i32::MIN as u32, u32::MAX, 7, i32::MAX as u32], "RGBA32I");
        }
        let w = 1023u32 | (5 << 20) | (2 << 30);
        if let Some(v) = one_texel(
            GL_RGB10_A2UI,
            GL_RGBA_INTEGER,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            &w.to_ne_bytes(),
            "usampler2D",
            "uvec4",
        ) {
            assert_eq!(v, [1023, 0, 5, 2], "RGB10_A2UI");
        }
    }

    /// An integer 2D array through texelFetch on an isampler2DArray: the
    /// integer path of a layered target.
    #[test]
    fn integer_textures_of_the_other_targets() {
        let _g = context();
        let mut t = 0;
        // SAFETY: plain GL calls; the texels outlive the uploads.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D_ARRAY, t);
            glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            let layers: [i8; 8] = [-1, 2, 3, 4, -5, 6, 7, 8];
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                GL_RGBA8I as GLint,
                1,
                1,
                2,
                0,
                GL_RGBA_INTEGER,
                GL_BYTE,
                layers.as_ptr().cast(),
            );
            if err() != GL_NO_ERROR {
                eprintln!("RGBA8I: not stored on this back end; skipped");
                return;
            }
        }
        let src = vs("uniform highp isampler2DArray s;", "ivec4", "texelFetch(s, ivec3(0, 0, int(a.x)), 0)");
        if let Some(got) = try_ints(&src, &[("s", 0)], &[[0.0; 4], [1.0, 0.0, 0.0, 0.0]]) {
            assert_eq!(got, [[-1i32 as u32, 2, 3, 4], [-5i32 as u32, 6, 7, 8]]);
        }
    }

    /* ---- sampler state ------------------------------------------------------- */

    /// A sampler object bound to the unit replaces the texture's filters and
    /// wraps (ES 3.0.6 3.8.2); the texture swizzle reorders the result
    /// (3.8.14.2); a mipmap filter on a one-level texture leaves it
    /// incomplete, which reads (0, 0, 0, 1) (3.8.13).
    #[test]
    fn sampler_objects_the_swizzle_and_incompleteness() {
        let _g = context();
        let quad: Vec<[f32; 4]> = (0..4).map(|i| [i as f32, 10.0, 20.0, 30.0]).collect();
        texture(GL_TEXTURE_2D, 0, GL_LINEAR, GL_LINEAR, GL_CLAMP_TO_EDGE);
        image_f(0, GL_TEXTURE_2D, 0, 2, 2, &quad);
        let src = vs("uniform sampler2D s;", "vec4", "texture(s, a.xy)");
        let centre = [[0.5, 0.5, 0.0, 0.0]];
        assert_eq!(floats(&src, &[("s", 0)], &centre)[0], [1.5, 10.0, 20.0, 30.0], "LINEAR");
        let mut smp = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenSamplers(1, &mut smp);
            glSamplerParameteri(smp, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glSamplerParameteri(smp, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glBindSampler(0, smp);
        }
        assert_eq!(
            floats(&src, &[("s", 0)], &centre)[0],
            [3.0, 10.0, 20.0, 30.0],
            "the sampler's NEAREST: texel (1, 1)"
        );
        // SAFETY: plain GL calls.
        unsafe {
            glBindSampler(0, 0);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_BLUE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_G, GL_ONE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_A, GL_ZERO as GLint);
        }
        assert_eq!(floats(&src, &[("s", 0)], &centre)[0], [20.0, 1.0, 20.0, 0.0], "swizzled");
        // SAFETY: plain GL calls.
        unsafe {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_RED as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_G, GL_GREEN as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_A, GL_ALPHA as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        }
        /* lod 1 minifies, which needs the missing level 1 */
        let lod = vs("uniform sampler2D s;", "vec4", "textureLod(s, a.xy, 1.0)");
        assert_eq!(floats(&lod, &[("s", 0)], &centre)[0], [0.0, 0.0, 0.0, 1.0], "incomplete");
    }

    /// Element k of a sampler array reads the unit its glUniform1i gave it
    /// (the array's binding plus k, in the IR and in the linked stage
    /// alike), here in the other order than the elements; and a depth
    /// texture read through a plain sampler2D is its depth in red, alpha 1
    /// (ES 3.0.6 3.8.14.1, "treated as RED").
    #[test]
    fn sampler_arrays_and_a_depth_texture() {
        let _g = context();
        texture(GL_TEXTURE_2D, 1, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        image_f(1, GL_TEXTURE_2D, 0, 1, 1, &[[11.0, 0.0, 0.0, 1.0]]);
        texture(GL_TEXTURE_2D, 2, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        image_f(2, GL_TEXTURE_2D, 0, 1, 1, &[[22.0, 0.0, 0.0, 1.0]]);
        let src = vs(
            "uniform sampler2D ss[2];",
            "vec4",
            "vec4(texture(ss[0], vec2(0.5)).x, texture(ss[1], vec2(0.5)).x, 0.0, 0.0)",
        );
        assert_eq!(floats(&src, &[("ss[0]", 2), ("ss[1]", 1)], &[[0.0; 4]])[0], [22.0, 11.0, 0.0, 0.0]);
        texture(GL_TEXTURE_2D, 0, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        // SAFETY: plain GL calls; the depth outlives the upload.
        unsafe {
            let d = [0.25f32];
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT32F as GLint,
                1,
                1,
                0,
                GL_DEPTH_COMPONENT,
                GL_FLOAT,
                d.as_ptr().cast(),
            );
        }
        assert_eq!(err(), GL_NO_ERROR);
        let src = vs("uniform highp sampler2D s;", "vec4", "texture(s, vec2(0.5))");
        let v = floats(&src, &[("s", 0)], &[[0.0; 4]])[0];
        assert_eq!((v[0], v[3]), (0.25, 1.0), "depth in red: {v:?}");
    }

    /// A sampler no texture was ever bound to reads the default texture,
    /// which has no image: (0, 0, 0, 1), as the draw's.
    #[test]
    fn an_unbound_sampler_reads_opaque_black() {
        let _g = context();
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE3);
            glBindTexture(GL_TEXTURE_3D, 0);
            glActiveTexture(GL_TEXTURE0);
        }
        let src = vs("uniform highp sampler3D s;", "vec4", "textureLod(s, a.xyz, 0.0)");
        if let Some(v) = try_floats(&src, &[("s", 3)], &[[0.5; 4]]) {
            assert_eq!(v[0], [0.0, 0.0, 0.0, 1.0], "3D");
        }
        let src = vs("uniform samplerCube s;", "vec4", "textureLod(s, a.xyz, 0.0)");
        // SAFETY: plain GL calls.
        unsafe {
            glActiveTexture(GL_TEXTURE3);
            glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
            glActiveTexture(GL_TEXTURE0);
        }
        assert_eq!(floats(&src, &[("s", 3)], &[[0.5; 4]])[0], [0.0, 0.0, 0.0, 1.0], "cube");
    }

    /* ---- refusals ------------------------------------------------------------ */

    /// What the capture does not evaluate is refused whole: GL_INVALID_OPERATION,
    /// nothing written (the guard is checked in `run`). The reasons, by name,
    /// are pinned in `src/gl/tf_capture_tests.rs`. A look-up a GPU's code
    /// generator refuses never gets as far as a draw; that counts as refused
    /// too.
    #[test]
    fn non_depth_shadow_compressed_and_mismatched_samplers_are_refused() {
        let _g = context();
        texture(GL_TEXTURE_2D, 0, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        image_f(0, GL_TEXTURE_2D, 0, 1, 1, &[[1.0; 4]]);
        let refused = |o: Outcome| matches!(o, Outcome::Refused(GL_INVALID_OPERATION) | Outcome::NotCompiled);
        /* textureGrad is captured now; a GPU whose code generator lacks it never gets here */
        let grad = vs("uniform sampler2D s;", "vec4", "textureGrad(s, a.xy, vec2(0.1), vec2(0.1))");
        if let Some(v) = try_floats(&grad, &[("s", 0)], &[[0.5; 4]]) {
            assert_eq!(v[0], [1.0; 4], "textureGrad");
        }
        /* a shadow sampler on a texture that is not a depth texture is undefined in GL */
        let shadow = vs("uniform highp sampler2DShadow s;", "vec4", "vec4(texture(s, a.xyz))");
        assert!(refused(run(&shadow, &[("s", 0)], &[[0.5; 4]])), "sampler2DShadow");
        let int = vs("uniform highp isampler2D s;", "ivec4", "texture(s, a.xy)");
        assert!(refused(run(&int, &[("s", 0)], &[[0.5; 4]])), "isampler2D on RGBA16F");
        /* ETC2: one 4x4 block of zeros */
        texture(GL_TEXTURE_2D, 1, GL_NEAREST, GL_NEAREST, GL_CLAMP_TO_EDGE);
        // SAFETY: plain GL calls; the block outlives the upload.
        unsafe {
            glActiveTexture(GL_TEXTURE1);
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 4, 4, 0, 8, [0u8; 8].as_ptr().cast());
            glActiveTexture(GL_TEXTURE0);
        }
        assert_eq!(err(), GL_NO_ERROR);
        let plain = vs("uniform sampler2D s;", "vec4", "texture(s, a.xy)");
        assert_eq!(
            run(&plain, &[("s", 1)], &[[0.5; 4]]),
            Outcome::Refused(GL_INVALID_OPERATION),
            "a compressed texture"
        );
    }
}
