// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of the uniform types glmark2-es2 sets, on the Mali back
//! end (plan #367, item test-uniform-types). The shaders are written for this
//! test; no glmark2 text is copied.
//!
//! glmark2-es2 sets a mat4 (ModelViewProjectionMatrix), a mat3 (NormalMatrix)
//! through glUniformMatrix*fv, vec3 and vec4 (light position, colours), a
//! float (time) and int sampler uniforms with glUniform1i. Checked here:
//! glGetUniformLocation, glGetActiveUniform (name, type, size) and
//! glGetUniformfv/iv round-trip every one of them; a draw sees an updated
//! vec4 colour (the host has no Mali OSAL, so no shader executes and no pixel
//! is written: what is compared is the fragment uniform block `mali_draw` is
//! handed, which holds the colour's words); a uniform keeps its value across
//! glUseProgram switches.
//!
//! A tools/check.sh row (the Mali GL-on-mock pins).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_uniforms
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::{mali_draw, yakogl_backend_mali};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
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

/// What the back end was handed for the draw's texture, and what the Mali
/// made of the draw.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct Seen {
    type_: u8,
    width: u32,
    mip_levels: u8,
    mip_filter: u8,
    incomplete: u8,
    mali: i32,
}

/// The fragment uniform block of each draw.
static FS_WORDS: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());
static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let t = &d.textures[0];
    if !d.fs_uniform_cpu.is_null() {
        // SAFETY: the GL core hands `fs_uniform_dwords` words at the cpu pointer.
        let w = core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, d.fs_uniform_dwords as usize);
        FS_WORDS.lock().unwrap().push(w.to_vec());
    }
    let mali = mali_draw(d);
    SEEN.lock().unwrap().push(Seen {
        type_: t.type_,
        width: t.width,
        mip_levels: t.mip_levels,
        mip_filter: t.mip_filter,
        incomplete: t.incomplete,
        mali,
    });
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

/// The Mali's table with the stand-in's own entries (a draw that records, a
/// heap base, no hardware to start or wait for).
fn stand_in() -> &'static YakoglBackend {
    let m = &yakogl_backend_mali;
    Box::leak(Box::new(YakoglBackend {
        name: m.name,
        native_attrib_formats: m.native_attrib_formats,
        rt_rgba8888_is_bgra: m.rt_rgba8888_is_bgra,
        mip_level_align: m.mip_level_align,
        compressed_blocks_in_rows: m.compressed_blocks_in_rows,
        native_layered_textures: m.native_layered_textures,
        front_facing_shader_variant: m.front_facing_shader_variant,
        etc1_decoded_at_upload: m.etc1_decoded_at_upload,
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
        max_instance_divisor: m.max_instance_divisor,
        wait_kick: None,
        samples_texel_copies: m.samples_texel_copies,
        attachment_gone: None,
        depth_resolve: None,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: m.generate_mipmap,
        return_mipmap_chain: m.return_mipmap_chain,
        linear_row_align: m.linear_row_align,
        uniforms_host_only: m.uniforms_host_only,
        ds_write: None,
        stencil_resolve: None,
        max_samples: m.max_samples,
    }))
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = stand_in();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    SEEN.lock().unwrap().clear();
    FS_WORDS.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program(vs: &str, fs: &str, attribs: &[&core::ffi::CStr]) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        for (i, a) in attribs.iter().enumerate() {
            glBindAttribLocation(p, i as GLuint, a.as_ptr());
        }
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        p
    }
}

/// An RGBA8 texture as the colour attachment of a framebuffer.
fn target(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

const VS: &str = "attribute vec3 a_pos;
uniform mat4 u_mvp;
uniform mat3 u_nrm;
uniform vec3 u_light;
uniform float u_time;
varying mediump float v_d;
void main()
{
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_d = dot(u_nrm * u_light, vec3(0.0, 0.0, 1.0)) * u_time;
}
";
const FS: &str = "precision mediump float;
uniform vec4 u_color;
uniform sampler2D u_tex;
varying mediump float v_d;
void main()
{
    gl_FragColor = u_color * (0.5 + 0.5 * v_d) + texture2D(u_tex, vec2(0.5, 0.5)) * 0.0;
}
";
/// A second, unrelated program with one vec4 of its own.
const FS2: &str = "precision mediump float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color;
}
";
const VS2: &str = "attribute vec3 a_pos;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
}
";

fn loc(p: GLuint, n: &core::ffi::CStr) -> GLint {
    // SAFETY: plain GL call.
    unsafe { glGetUniformLocation(p, n.as_ptr()) }
}

fn getf(p: GLuint, l: GLint, n: usize) -> Vec<f32> {
    let mut v = vec![f32::NAN; n];
    // SAFETY: room for `n` floats.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    v
}

fn geti(p: GLuint, l: GLint) -> GLint {
    let mut v: GLint = -99;
    // SAFETY: room for one int.
    unsafe { glGetUniformiv(p, l, &mut v) };
    v
}

fn draw6() {
    // SAFETY: the array is static.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
}

static POS: [f32; 18] =
    [-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0];

fn full_program() -> GLuint {
    program(VS, FS, &[c"a_pos"])
}

#[test]
fn active_uniforms_report_name_type_size() {
    let _g = context();
    let p = full_program();
    let want: [(&str, GLenum); 6] = [
        ("u_mvp", GL_FLOAT_MAT4),
        ("u_nrm", GL_FLOAT_MAT3),
        ("u_light", GL_FLOAT_VEC3),
        ("u_time", GL_FLOAT),
        ("u_color", GL_FLOAT_VEC4),
        ("u_tex", GL_SAMPLER_2D),
    ];
    let mut n: GLint = 0;
    // SAFETY: one int out.
    unsafe { glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut n) };
    assert_eq!(n, 6, "six active uniforms");
    let mut seen = std::collections::HashMap::new();
    for i in 0..n as GLuint {
        let (mut len, mut size, mut ty) = (0, 0, 0);
        let mut name = [0u8; 64];
        // SAFETY: the buffer is 64 bytes.
        unsafe { glGetActiveUniform(p, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr().cast()) };
        let s = String::from_utf8(name[..len as usize].to_vec()).unwrap();
        assert!(loc(p, &CString::new(s.clone()).unwrap()) >= 0, "{s} has a location");
        seen.insert(s, (ty, size));
    }
    for (name, ty) in &want {
        assert_eq!(seen.get(*name), Some(&(*ty, 1)), "{name}: type and size");
    }
    assert_eq!(loc(p, c"u_missing"), -1);
    assert_eq!(err(), GL_NO_ERROR);
}

#[test]
fn uniforms_round_trip_through_getters() {
    let _g = context();
    let p = full_program();
    let mvp: [f32; 16] = core::array::from_fn(|i| i as f32 + 0.5);
    let nrm: [f32; 9] = core::array::from_fn(|i| i as f32 * 2.0 - 3.0);
    // SAFETY: arrays sized for the calls; the program in use has the names.
    unsafe {
        glUniformMatrix4fv(loc(p, c"u_mvp"), 1, 0, mvp.as_ptr());
        glUniformMatrix3fv(loc(p, c"u_nrm"), 1, 0, nrm.as_ptr());
        glUniform3f(loc(p, c"u_light"), 1.5, -2.0, 0.25);
        glUniform4f(loc(p, c"u_color"), 0.1, 0.2, 0.3, 0.4);
        glUniform1f(loc(p, c"u_time"), 12.5);
        glUniform1i(loc(p, c"u_tex"), 3);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(getf(p, loc(p, c"u_mvp"), 16), mvp);
    assert_eq!(getf(p, loc(p, c"u_nrm"), 9), nrm);
    assert_eq!(getf(p, loc(p, c"u_light"), 3), [1.5, -2.0, 0.25]);
    assert_eq!(getf(p, loc(p, c"u_color"), 4), [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(getf(p, loc(p, c"u_time"), 1), [12.5]);
    assert_eq!(geti(p, loc(p, c"u_tex")), 3);
    // The same through the int getter for a float uniform (rounded).
    assert_eq!(geti(p, loc(p, c"u_time")), 13);
    assert_eq!(err(), GL_NO_ERROR);

    // A second set replaces the first.
    let m3: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    // SAFETY: nine floats.
    unsafe { glUniformMatrix3fv(loc(p, c"u_nrm"), 1, 0, m3.as_ptr()) };
    assert_eq!(getf(p, loc(p, c"u_nrm"), 9), m3);
}

#[test]
fn wrong_setter_raises_invalid_operation() {
    let _g = context();
    let p = full_program();
    // SAFETY: plain GL calls.
    unsafe {
        glUniform4f(loc(p, c"u_time"), 1.0, 2.0, 3.0, 4.0);
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "vec4 setter on a float");
    // SAFETY: plain GL call.
    unsafe { glUniform1f(loc(p, c"u_tex"), 1.0) };
    assert_eq!(err(), GL_INVALID_OPERATION, "float setter on a sampler");
    // Location -1 is silently ignored.
    // SAFETY: plain GL call.
    unsafe { glUniform1f(-1, 1.0) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn bits(v: f32) -> u32 {
    v.to_bits()
}

#[test]
fn draw_sees_updated_colour() {
    let _g = context();
    let (fbo, tex) = target(8, 8);
    let p = full_program();
    let id: [f32; 16] = core::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
    let n3: [f32; 9] = core::array::from_fn(|i| if i % 4 == 0 { 1.0 } else { 0.0 });
    // SAFETY: arrays sized for the calls.
    unsafe {
        glUniformMatrix4fv(loc(p, c"u_mvp"), 1, 0, id.as_ptr());
        glUniformMatrix3fv(loc(p, c"u_nrm"), 1, 0, n3.as_ptr());
        glUniform3f(loc(p, c"u_light"), 0.0, 0.0, 1.0);
        glUniform1f(loc(p, c"u_time"), 1.0);
        glUniform1i(loc(p, c"u_tex"), 0);
        glUniform4f(loc(p, c"u_color"), 0.125, 0.25, 0.5, 1.0);
    }
    draw6();
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL call.
    unsafe { glUniform4f(loc(p, c"u_color"), 0.75, 0.0625, 0.375, 0.5) };
    draw6();
    assert_eq!(err(), GL_NO_ERROR);
    let blocks = FS_WORDS.lock().unwrap().clone();
    assert_eq!(blocks.len(), 2, "two draws reached the back end");
    let has = |b: &Vec<u32>, c: [f32; 4]| b.windows(4).any(|w| w == c.map(bits));
    // The compiler may fold the constants of the colour product into the
    // block, so look for the colour's four words as a run.
    let (a, b) = (has(&blocks[0], [0.125, 0.25, 0.5, 1.0]), has(&blocks[1], [0.75, 0.0625, 0.375, 0.5]));
    assert!(a, "first draw's block holds the first colour: {:08x?}", blocks[0]);
    assert!(b, "second draw's block holds the updated colour: {:08x?}", blocks[1]);
    assert!(!has(&blocks[1], [0.125, 0.25, 0.5, 1.0]), "the old colour is gone");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}

#[test]
fn uniform_keeps_value_across_use_program() {
    let _g = context();
    let a = full_program();
    // SAFETY: plain GL calls.
    unsafe { glUniform4f(loc(a, c"u_color"), 0.1, 0.2, 0.3, 0.4) };
    let b = program(VS2, FS2, &[c"a_pos"]);
    // SAFETY: plain GL calls.
    unsafe { glUniform4f(loc(b, c"u_color"), 0.9, 0.8, 0.7, 0.6) };
    for _ in 0..2 {
        // SAFETY: plain GL calls.
        unsafe { glUseProgram(a) };
        assert_eq!(getf(a, loc(a, c"u_color"), 4), [0.1, 0.2, 0.3, 0.4]);
        // SAFETY: plain GL calls.
        unsafe { glUseProgram(b) };
        assert_eq!(getf(b, loc(b, c"u_color"), 4), [0.9, 0.8, 0.7, 0.6]);
        assert_eq!(getf(a, loc(a, c"u_color"), 4), [0.1, 0.2, 0.3, 0.4], "a keeps its value while b is current");
    }
    // The set goes to the current program only.
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(a);
        glUniform4f(loc(a, c"u_color"), 0.5, 0.5, 0.5, 0.5);
    }
    assert_eq!(getf(b, loc(b, c"u_color"), 4), [0.9, 0.8, 0.7, 0.6]);
    assert_eq!(err(), GL_NO_ERROR);
}
