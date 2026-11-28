// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of the Mali back end: the index types and primitive
//! modes glmark2-es2 draws with (plan #367, item test-index-draw).
//!
//! glmark2's model scenes call `glDrawElements(GL_TRIANGLES,
//! GL_UNSIGNED_SHORT)` from an element array buffer at a non-zero offset;
//! other scenes use `glDrawArrays` with GL_TRIANGLES, GL_TRIANGLE_STRIP,
//! GL_TRIANGLE_FAN and GL_LINES. Covered: ushort and ubyte indices from an
//! element VBO and from a client-side index pointer, a non-zero `first` and
//! a non-zero index offset, strip and fan, and a draw with count 0, which
//! raises no error and reaches no back end. The shaders are written for this
//! test and compile on the Mali online compiler.
//!
//! Level reached: GL core plus the Mali back end's `mali_draw`. The host has
//! no Mali OSAL, so the stand-in back end (as in
//! `tests/mali_glmark2_scene_draw.rs`) hands the draw to `mali_draw`, which
//! ends at `-ENODEV`; no shader executes and no pixel is written. Checked
//! after each call: glGetError, what the back end was handed (primitive,
//! index count, the index stream the core staged, the first attribute's
//! offset or staged bytes), and glReadPixels (error-free; the target keeps
//! its clear colour, which for count 0 is the "no pixels change" check).
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_index_draw
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::backend::{mali_draw, yakogl_backend_mali};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENODEV;

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

/// What the back end was handed for one draw.
#[derive(Clone, Debug)]
struct Seen {
    prim: u32,
    index_count: u32,
    index_size: u8,
    indexed: bool,
    /// The widened index stream (u16 each) the core staged.
    indices: Vec<u16>,
    attrib0_offset: u32,
    attrib0_cpu_bytes: u32,
    /// The first float of attribute 0 when the core staged it (`cpu` set).
    attrib0_cpu_first: Option<f32>,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        /* a settled glClear, not an application draw */
        return 0;
    }
    let mali = mali_draw(d);
    /* staged indices (a client pointer, a widened ubyte array) are in the
     * core's scratch; a ushort element buffer is fetched in place, and the
     * stand-in's GPU address is its CPU pointer */
    let (p, n) = if d.index_cpu.is_null() {
        (d.index_buf_gpu as *const u8, d.index_count as usize * 2)
    } else {
        (d.index_cpu as *const u8, d.index_bytes as usize)
    };
    let indices: Vec<u16> = if d.index_buf_gpu == 0 {
        Vec::new()
    } else {
        core::slice::from_raw_parts(p, n).chunks_exact(2).map(|q| u16::from_ne_bytes([q[0], q[1]])).collect()
    };
    let a = &d.attribs[0];
    SEEN.lock().unwrap().push(Seen {
        prim: d.prim,
        index_count: d.index_count,
        index_size: d.index_size,
        indexed: d.index_buf_gpu != 0,
        indices,
        attrib0_offset: a.offset,
        attrib0_cpu_bytes: a.cpu_bytes,
        attrib0_cpu_first: if a.cpu.is_null() { None } else { Some(*(a.cpu as *const f32)) },
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
void main()
{
    gl_Position = vec4(a_pos, 1.0);
}
";
const FS: &str = "precision mediump float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color;
}
";

fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), 1.0, 0.5, 0.25, 1.0);
        p
    }
}

/// Eight vertices of x = -1 + i/4 (so vertex i's x identifies it), 3 floats.
fn verts() -> Vec<f32> {
    (0..8).flat_map(|i| [-1.0 + i as f32 / 4.0, (i % 2) as f32 - 0.5, 0.0]).collect()
}

struct Rig {
    fbo: GLuint,
    tex: GLuint,
    prog: GLuint,
    vbo: GLuint,
    ibo: GLuint,
    /// The client array (kept alive for the draws).
    data: Vec<f32>,
}

impl Rig {
    fn new() -> Rig {
        let (fbo, tex) = target(8, 8);
        let prog = program(VS, FS);
        let data = verts();
        let (mut vbo, mut ibo) = (0, 0);
        // SAFETY: room for one name each; the data covers the sizes given.
        unsafe {
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(GL_ARRAY_BUFFER, (data.len() * 4) as GLsizeiptr, data.as_ptr().cast(), GL_STATIC_DRAW);
            glGenBuffers(1, &mut ibo);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glEnableVertexAttribArray(0);
        }
        assert_eq!(err(), GL_NO_ERROR, "rig set-up");
        Rig { fbo, tex, prog, vbo, ibo, data }
    }

    /// Attribute 0 from the client array.
    fn client_attrib(&self) {
        // SAFETY: `data` outlives the draws.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, self.data.as_ptr() as *const c_void);
        }
    }

    /// Attribute 0 from the VBO, starting `off` bytes in.
    fn vbo_attrib(&self, off: usize) {
        // SAFETY: plain GL calls; the offset is inside the buffer.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, off as *const c_void);
        }
    }

    fn upload_indices(&self, bytes: &[u8]) {
        // SAFETY: `bytes` covers the size given.
        unsafe {
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, self.ibo);
            glBufferData(GL_ELEMENT_ARRAY_BUFFER, bytes.len() as GLsizeiptr, bytes.as_ptr().cast(), GL_STATIC_DRAW);
        }
    }

    fn unbind_indices(&self) {
        // SAFETY: plain GL call.
        unsafe { glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0) };
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // SAFETY: names this rig made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glDisableVertexAttribArray(0);
            glUseProgram(0);
            glDeleteProgram(self.prog);
            glDeleteFramebuffers(1, &self.fbo);
            glDeleteTextures(1, &self.tex);
            glDeleteBuffers(1, &self.vbo);
            glDeleteBuffers(1, &self.ibo);
        }
    }
}

/// What one draw call did: its GL error, the read-back error and pixel, and
/// what reached the back end.
struct Outcome {
    draw_err: GLenum,
    read_err: GLenum,
    px: [u8; 4],
    /// The same pixel before the call.
    px0: [u8; 4],
    seen: Vec<Seen>,
}

fn run(f: impl FnOnce()) -> Outcome {
    let mut px0 = [0u8; 4];
    // SAFETY: one pixel into `px0`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px0.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR);
    SEEN.lock().unwrap().clear();
    f();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let draw_err = err();
    let mut px = [0u8; 4];
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    let read_err = err();
    Outcome { draw_err, read_err, px, px0, seen: core::mem::take(&mut *SEEN.lock().unwrap()) }
}

fn ok(o: &Outcome) -> &Seen {
    assert_eq!((o.draw_err, o.read_err), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!(o.seen.len(), 1, "one draw reached the back end");
    let s = &o.seen[0];
    assert_eq!(s.mali, -ENODEV, "the Mali takes the draw");
    s
}

fn u16_bytes(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

#[test]
fn ushort_elements_from_a_vbo_at_an_offset() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    // glmark2 models: ushort indices in an element buffer, drawn from a
    // non-zero byte offset (here 6 bytes: the fourth index on).
    r.upload_indices(&u16_bytes(&[0, 1, 2, 3, 4, 5, 6, 7]));
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 6 as *const c_void) });
    let s = ok(&o);
    assert_eq!((s.prim, s.index_count, s.index_size, s.indexed), (GLES_PRIM_TRIANGLES, 3, 2, true));
    assert_eq!(s.indices, [3, 4, 5], "the indices from byte offset 6");
    assert_eq!(o.px, o.px0, "nothing is drawn on the host");
    // the offset 0 draw of the same buffer
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null()) });
    assert_eq!(ok(&o).indices, [0, 1, 2, 3, 4, 5]);
}

#[test]
fn ubyte_elements_from_a_vbo_at_an_offset() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    r.upload_indices(&[7, 6, 5, 4, 3, 2, 1, 0]);
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, 2 as *const c_void) });
    let s = ok(&o);
    assert_eq!((s.prim, s.index_count, s.indexed), (GLES_PRIM_TRIANGLES, 3, true));
    assert_eq!(s.index_size, 2, "ubyte indices are widened to ushort");
    assert_eq!(s.indices, [5, 4, 3], "the indices from byte offset 2");
}

#[test]
fn ushort_and_ubyte_elements_from_a_client_pointer() {
    let _g = context();
    let r = Rig::new();
    r.client_attrib();
    r.unbind_indices();
    let us: [u16; 6] = [4, 5, 6, 0, 1, 2];
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, us.as_ptr().cast()) });
    let s = ok(&o);
    assert_eq!((s.prim, s.index_count, s.index_size), (GLES_PRIM_TRIANGLES, 6, 2));
    assert_eq!(s.indices, [4, 5, 6, 0, 1, 2]);
    // a client pointer offset into the array (what the offset means with no buffer)
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, us.as_ptr().add(3).cast()) });
    assert_eq!(ok(&o).indices, [0, 1, 2]);
    let ub: [u8; 6] = [1, 3, 5, 0, 2, 4];
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, ub.as_ptr().cast()) });
    let s = ok(&o);
    assert_eq!((s.index_count, s.index_size), (6, 2));
    assert_eq!(s.indices, [1, 3, 5, 0, 2, 4]);
}

#[test]
fn elements_with_every_mode_glmark2_uses() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    r.upload_indices(&u16_bytes(&[0, 1, 2, 3, 4, 5, 6, 7]));
    for (mode, prim, n) in [
        (GL_TRIANGLES, GLES_PRIM_TRIANGLES, 6),
        (GL_TRIANGLE_STRIP, GLES_PRIM_TRI_STRIP, 6),
        (GL_TRIANGLE_FAN, GLES_PRIM_TRI_FAN, 6),
        (GL_LINES, GLES_PRIM_LINES, 6),
    ] {
        let o = run(|| unsafe { glDrawElements(mode, n, GL_UNSIGNED_SHORT, core::ptr::null()) });
        let s = ok(&o);
        assert_eq!((s.prim, s.index_count), (prim, n as u32), "mode {mode:#x}");
        assert_eq!(s.indices, [0, 1, 2, 3, 4, 5], "mode {mode:#x}");
    }
}

#[test]
fn arrays_triangles_strip_fan_and_lines() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    for (mode, prim, n) in [
        (GL_TRIANGLES, GLES_PRIM_TRIANGLES, 6),
        (GL_TRIANGLE_STRIP, GLES_PRIM_TRI_STRIP, 8),
        (GL_TRIANGLE_FAN, GLES_PRIM_TRI_FAN, 8),
        (GL_LINES, GLES_PRIM_LINES, 4),
    ] {
        let o = run(|| unsafe { glDrawArrays(mode, 0, n) });
        let s = ok(&o);
        assert_eq!((s.prim, s.index_count, s.indexed), (prim, n as u32, false), "mode {mode:#x}");
        assert_eq!(o.px, o.px0);
    }
}

#[test]
fn arrays_with_a_non_zero_first_from_a_vbo() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    let o = run(|| unsafe { glDrawArrays(GL_TRIANGLES, 3, 3) });
    let s = ok(&o);
    assert_eq!((s.index_count, s.indexed), (3, false));
    assert_eq!(s.attrib0_offset, 3 * 12, "`first` is folded into the attribute offset");
    // with a buffer offset as well (what glmark2's interleaved layouts do)
    r.vbo_attrib(12);
    let o = run(|| unsafe { glDrawArrays(GL_TRIANGLE_STRIP, 2, 4) });
    let s = ok(&o);
    assert_eq!((s.prim, s.index_count), (GLES_PRIM_TRI_STRIP, 4));
    assert_eq!(s.attrib0_offset, 12 + 2 * 12);
}

#[test]
fn arrays_with_a_non_zero_first_from_a_client_array() {
    let _g = context();
    let r = Rig::new();
    r.client_attrib();
    let o = run(|| unsafe { glDrawArrays(GL_TRIANGLE_FAN, 2, 4) });
    let s = ok(&o);
    assert_eq!((s.prim, s.index_count), (GLES_PRIM_TRI_FAN, 4));
    // the staged copy starts at vertex 2: x = -1 + 2/4
    assert_eq!(s.attrib0_cpu_first, Some(-0.5), "the upload starts at `first`");
    assert!(s.attrib0_cpu_bytes >= 4 * 12);
}

#[test]
fn elements_with_client_attribs_stage_to_the_largest_index() {
    let _g = context();
    let r = Rig::new();
    r.client_attrib();
    r.upload_indices(&u16_bytes(&[5, 6, 7, 5, 7, 6]));
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null()) });
    let s = ok(&o);
    assert_eq!(s.indices, [5, 6, 7, 5, 7, 6]);
    // indices address vertices absolutely: the array is staged from 0
    assert_eq!(s.attrib0_cpu_first, Some(-1.0));
    assert!(s.attrib0_cpu_bytes >= 8 * 12, "vertices 0..=7 are staged");
}

#[test]
fn count_zero_raises_no_error_and_changes_no_pixel() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    for mode in [GL_TRIANGLES, GL_TRIANGLE_STRIP, GL_TRIANGLE_FAN, GL_LINES] {
        let o = run(|| unsafe { glDrawArrays(mode, 0, 0) });
        assert_eq!((o.draw_err, o.read_err), (GL_NO_ERROR, GL_NO_ERROR), "arrays {mode:#x}");
        assert!(o.seen.is_empty(), "no draw reaches the back end");
        assert_eq!(o.px, o.px0);
    }
    r.upload_indices(&u16_bytes(&[0, 1, 2]));
    for ty in [GL_UNSIGNED_SHORT, GL_UNSIGNED_BYTE] {
        let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 0, ty, core::ptr::null()) });
        assert_eq!((o.draw_err, o.read_err), (GL_NO_ERROR, GL_NO_ERROR), "elements {ty:#x}");
        assert!(o.seen.is_empty());
        assert_eq!(o.px, o.px0);
    }
    r.unbind_indices();
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 0, GL_UNSIGNED_SHORT, core::ptr::null()) });
    assert_eq!((o.draw_err, o.read_err), (GL_NO_ERROR, GL_NO_ERROR), "client pointer, count 0");
    assert!(o.seen.is_empty());
}

#[test]
fn bad_arguments_raise_the_gl_errors() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    r.upload_indices(&u16_bytes(&[0, 1, 2]));
    let o = run(|| unsafe { glDrawArrays(GL_TRIANGLES, 0, -1) });
    assert_eq!(o.draw_err, GL_INVALID_VALUE);
    let o = run(|| unsafe { glDrawArrays(GL_TRIANGLES, -1, 3) });
    assert_eq!(o.draw_err, GL_INVALID_VALUE);
    let o = run(|| unsafe { glDrawArrays(0x0007, 0, 3) });
    assert_eq!(o.draw_err, GL_INVALID_ENUM, "GL_QUADS");
    let o = run(|| unsafe { glDrawElements(GL_TRIANGLES, 3, GL_FLOAT, core::ptr::null()) });
    assert_eq!(o.draw_err, GL_INVALID_ENUM, "a float index type");
    assert!(o.seen.is_empty());
}

#[test]
fn elements_then_arrays_in_one_frame() {
    let _g = context();
    let r = Rig::new();
    r.vbo_attrib(0);
    r.upload_indices(&u16_bytes(&[0, 1, 2, 3]));
    let o = run(|| unsafe {
        glDrawElements(GL_TRIANGLE_STRIP, 4, GL_UNSIGNED_SHORT, core::ptr::null());
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 2 as *const c_void);
    });
    assert_eq!((o.draw_err, o.read_err), (GL_NO_ERROR, GL_NO_ERROR));
    assert_eq!(o.seen.len(), 3);
    assert_eq!((o.seen[0].prim, o.seen[0].indexed), (GLES_PRIM_TRI_STRIP, true));
    assert_eq!((o.seen[1].prim, o.seen[1].indexed), (GLES_PRIM_TRIANGLES, false));
    assert_eq!(o.seen[2].indices, [1, 2, 3]);
}
