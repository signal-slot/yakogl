// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test: the draws of glmark2-es2's `buffer` scene on the Mali
//! back end (plan #367). The scene draws a grid of vertices with
//! glDrawElements from a vertex buffer and an index buffer, rewrites part of
//! the vertex buffer with glBufferSubData between frames, and varies the
//! layout (`interleave`), the usage (`update-method` map/subdata, STREAM or
//! STATIC draw) and `use-vbo=false` (client-side arrays). The shaders here
//! are written for this test (a position and a colour attribute passed
//! through); no glmark2 text is copied.
//!
//! Level reached: link plus draw submission, as `mali_glmark2_scene_draw.rs`.
//! The host has no Mali OSAL, so the stand-in back end hands each draw to
//! `mali_draw` (which ends at `-ENODEV`) and no shader executes: there is no
//! pixel to read back. So "the read-back reflects the update" is checked one
//! step earlier, at the only place a stale buffer can show on the host: the
//! stand-in copies, at draw time, what the back end was handed for every
//! vertex attribute and for the indices (the bytes behind `buffer_gpu` or
//! `cpu`, which in this OSAL are host memory), and the test compares those
//! copies with what the application wrote last. A draw that kept the old
//! contents, an offset or a stride of the wrong layout, or client arrays that
//! were not re-read would differ. glReadPixels is called and must raise no
//! error; its values are not checked.
//!
//! Not a tools/check.sh row.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_buffer_draw
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
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

/// What the back end was handed by one draw, copied at draw time.
#[derive(Clone, Debug, Default)]
struct Seen {
    mali: i32,
    index_count: u32,
    /// The indices as the back end can read them.
    indices: Vec<u16>,
    /// Per attribute (by `vi_reg` order of the list): components, whether it
    /// came from a buffer object in place, and the floats of vertices
    /// `0..VERTS` (`components` each).
    attribs: Vec<(u8, bool, Vec<f32>)>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Vertices copied per attribute: the whole grid.
const VERTS: usize = (GRID + 1) * (GRID + 1);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let mali = mali_draw(d);
    let mut s = Seen { mali, index_count: d.index_count, ..Default::default() };
    if d.index_buf_gpu != 0 {
        let p = d.index_buf_gpu as *const u16;
        s.indices = (0..d.index_count as usize).map(|i| p.add(i).read_unaligned()).collect();
    } else if !d.index_cpu.is_null() {
        let p = d.index_cpu as *const u16;
        s.indices = (0..d.index_count as usize).map(|i| p.add(i).read_unaligned()).collect();
    }
    for a in &d.attribs[..d.num_attribs as usize] {
        let in_place = a.cpu.is_null();
        let base = if in_place { (a.buffer_gpu as *const u8).add(a.offset as usize) } else { a.cpu as *const u8 };
        let mut f = Vec::new();
        for v in 0..VERTS {
            for c in 0..a.components as usize {
                f.push((base.add(v * a.stride as usize + c * 4) as *const f32).read_unaligned());
            }
        }
        s.attribs.push((a.components, in_place, f));
    }
    SEEN.lock().unwrap().push(s);
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

/// The Mali's table with the stand-in's own entries.
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock serialises tests.
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

const VS: &str = "attribute vec3 a_pos;
attribute vec3 a_col;
varying mediump vec3 v_col;
void main()
{
    gl_Position = vec4(a_pos, 1.0);
    v_col = a_col;
}
";
const FS: &str = "precision mediump float;
varying mediump vec3 v_col;
void main()
{
    gl_FragColor = vec4(v_col, 1.0);
}
";

fn program() -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_col".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        p
    }
}

fn target() -> (GLuint, GLuint) {
    let (mut fbo, mut tex) = (0, 0);
    // SAFETY: room for one name each; null texels allocate the image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 8, 8);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR, "target set-up");
    (fbo, tex)
}

/* ---- the grid: GRID x GRID cells, (GRID+1)^2 vertices ---------------------- */

const GRID: usize = 3;
const NIDX: usize = GRID * GRID * 6;

/// Position (x, y, 0) of vertex `v`, and a colour that depends on `gen` so
/// that every rewrite is distinguishable.
fn pos(v: usize) -> [f32; 3] {
    let (x, y) = (v % (GRID + 1), v / (GRID + 1));
    [x as f32 / GRID as f32 * 2.0 - 1.0, y as f32 / GRID as f32 * 2.0 - 1.0, 0.0]
}
fn col(v: usize, gen: u32) -> [f32; 3] {
    [(v as f32 + 1.0) / 16.0, gen as f32 / 8.0, 0.5]
}
fn indices() -> Vec<u16> {
    let mut i = Vec::new();
    for cy in 0..GRID {
        for cx in 0..GRID {
            let a = (cy * (GRID + 1) + cx) as u16;
            let (b, c, d) = (a + 1, a + (GRID + 1) as u16, a + (GRID + 2) as u16);
            i.extend_from_slice(&[a, b, c, c, b, d]);
        }
    }
    assert_eq!(i.len(), NIDX);
    i
}
fn bytes(f: &[f32]) -> Vec<u8> {
    f.iter().flat_map(|x| x.to_ne_bytes()).collect()
}
/// `gen` per vertex: the interleaved array (pos, col) of the whole grid.
fn interleaved(gen: &[u32]) -> Vec<f32> {
    (0..VERTS).flat_map(|v| pos(v).into_iter().chain(col(v, gen[v]))).collect()
}

/// The floats the back end must have seen for attribute `k` (0 position, 1
/// colour) when vertex `v` is at generation `gen[v]`.
fn expect(k: usize, gen: &[u32]) -> Vec<f32> {
    (0..VERTS).flat_map(|v| if k == 0 { pos(v) } else { col(v, gen[v]) }).collect()
}

fn last_draw() -> Seen {
    let mut px = [0u8; 4];
    // SAFETY: one pixel into `px`.
    unsafe { glReadPixels(4, 4, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    assert_eq!(s.len(), 1, "one draw reached the back end");
    s.into_iter().next().unwrap()
}

/// Draw the grid with the arrays as currently bound and check the copy.
fn draw_and_check(client_indices: Option<&[u16]>, gen: &[u32], what: &str) -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: with a client index array the slice outlives the call; with an
    // element buffer bound the pointer is an offset.
    unsafe {
        let p = client_indices.map_or(core::ptr::null(), |i| i.as_ptr() as *const c_void);
        glDrawElements(GL_TRIANGLES, NIDX as GLsizei, GL_UNSIGNED_SHORT, p);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{what}: the draw");
    let s = last_draw();
    assert_eq!(s.mali, -ENODEV, "{what}: the Mali takes the draw");
    assert_eq!(s.index_count as usize, NIDX, "{what}");
    assert_eq!(s.indices, indices(), "{what}: indices");
    assert_eq!(s.attribs.len(), 2, "{what}: two attributes");
    assert_eq!(s.attribs[0].2, expect(0, gen), "{what}: positions");
    assert_eq!(s.attribs[1].2, expect(1, gen), "{what}: colours");
    s
}

fn new_buffer(target: GLenum, data: &[u8], usage: GLenum) -> GLuint {
    let mut b = 0;
    // SAFETY: room for one name; `data` is valid for its length.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(target, b);
        glBufferData(target, data.len() as GLsizeiptr, data.as_ptr().cast(), usage);
    }
    assert_eq!(err(), GL_NO_ERROR, "glBufferData");
    b
}

fn sub_data(target: GLenum, b: GLuint, offset: usize, data: &[u8]) {
    // SAFETY: the buffer is at least offset + len bytes.
    unsafe {
        glBindBuffer(target, b);
        glBufferSubData(target, offset as GLintptr, data.len() as GLsizeiptr, data.as_ptr().cast());
    }
    assert_eq!(err(), GL_NO_ERROR, "glBufferSubData");
}

fn attrib(loc: GLuint, n: GLint, stride: GLsizei, off: usize, ptr: *const c_void) {
    // SAFETY: with a buffer bound `ptr` is an offset; without, it is valid
    // client memory for the draw that follows.
    unsafe {
        glVertexAttribPointer(loc, n, GL_FLOAT, 0, stride, ptr.wrapping_byte_add(off));
        glEnableVertexAttribArray(loc);
    }
    assert_eq!(err(), GL_NO_ERROR, "glVertexAttribPointer");
}

fn cleanup(fbo: GLuint, tex: GLuint, bufs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
}

/// Rewrite vertices `from..to` of `gen` to `g`; returns the bytes of the
/// interleaved range and the colour-only range.
fn bump(gen: &mut [u32], from: usize, to: usize, g: u32) {
    for x in &mut gen[from..to] {
        *x = g;
    }
}

/// glmark2 `interleave=true`: one VBO of (position, colour) per vertex,
/// updated by glBufferSubData between draws, for both usages.
fn interleaved_case(usage: GLenum, name: &str) {
    let _g = context();
    let (fbo, tex) = target();
    program();
    let mut gen = vec![0u32; VERTS];
    let ib = new_buffer(GL_ELEMENT_ARRAY_BUFFER, &bytes_u16(&indices()), GL_STATIC_DRAW);
    let vb = new_buffer(GL_ARRAY_BUFFER, &bytes(&interleaved(&gen)), usage);
    attrib(0, 3, 24, 0, core::ptr::null());
    attrib(1, 3, 24, 12, core::ptr::null());

    let s = draw_and_check(None, &gen, &format!("{name} first draw"));
    assert!(s.attribs.iter().all(|a| a.1), "{name}: the attributes are fetched from the buffer in place");

    // Each frame rewrites a different run of vertices, as the scene's
    // `update-fraction` does, through the interleaved layout.
    for (frame, (from, to)) in [(0usize, 4usize), (4, 9), (2, 7), (0, VERTS)].into_iter().enumerate() {
        bump(&mut gen, from, to, frame as u32 + 1);
        let all = interleaved(&gen);
        sub_data(GL_ARRAY_BUFFER, vb, from * 24, &bytes(&all[from * 6..to * 6]));
        draw_and_check(None, &gen, &format!("{name} frame {}", frame + 1));
    }
    cleanup(fbo, tex, &[ib, vb]);
}

fn bytes_u16(i: &[u16]) -> Vec<u8> {
    i.iter().flat_map(|x| x.to_ne_bytes()).collect()
}

#[test]
fn interleaved_stream_draw_subdata() {
    interleaved_case(GL_STREAM_DRAW, "interleaved STREAM_DRAW");
}

#[test]
fn interleaved_static_draw_subdata() {
    interleaved_case(GL_STATIC_DRAW, "interleaved STATIC_DRAW");
}

/// glmark2 `interleave=false`: one VBO per attribute, each tightly packed.
fn separate_case(usage: GLenum, name: &str) {
    let _g = context();
    let (fbo, tex) = target();
    program();
    let mut gen = vec![0u32; VERTS];
    let ib = new_buffer(GL_ELEMENT_ARRAY_BUFFER, &bytes_u16(&indices()), GL_STATIC_DRAW);
    let pb = new_buffer(GL_ARRAY_BUFFER, &bytes(&expect(0, &gen)), GL_STATIC_DRAW);
    let cb = new_buffer(GL_ARRAY_BUFFER, &bytes(&expect(1, &gen)), usage);
    // glVertexAttribPointer captures the buffer bound at the call.
    // SAFETY: names made above.
    unsafe { glBindBuffer(GL_ARRAY_BUFFER, pb) };
    attrib(0, 3, 0, 0, core::ptr::null());
    // SAFETY: as above.
    unsafe { glBindBuffer(GL_ARRAY_BUFFER, cb) };
    attrib(1, 3, 0, 0, core::ptr::null());

    let s = draw_and_check(None, &gen, &format!("{name} first draw"));
    assert!(s.attribs.iter().all(|a| a.1), "{name}: in-place fetch");

    // Only the colour buffer changes; the position buffer must stay.
    for (frame, (from, to)) in [(0usize, 3usize), (5, 12), (1, VERTS)].into_iter().enumerate() {
        bump(&mut gen, from, to, frame as u32 + 1);
        let c = expect(1, &gen);
        sub_data(GL_ARRAY_BUFFER, cb, from * 12, &bytes(&c[from * 3..to * 3]));
        draw_and_check(None, &gen, &format!("{name} frame {}", frame + 1));
    }
    cleanup(fbo, tex, &[ib, pb, cb]);
}

#[test]
fn separate_stream_draw_subdata() {
    separate_case(GL_STREAM_DRAW, "separate STREAM_DRAW");
}

#[test]
fn separate_static_draw_subdata() {
    separate_case(GL_STATIC_DRAW, "separate STATIC_DRAW");
}

/// glmark2 `use-vbo=false`: client-side arrays (interleaved here, as the
/// scene's default) and a client index array, re-read at every draw.
#[test]
fn client_arrays_are_read_at_each_draw() {
    let _g = context();
    let (fbo, tex) = target();
    program();
    // SAFETY: no buffer object is bound for the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
    }
    let idx = indices();
    let mut gen = vec![0u32; VERTS];
    for frame in 0..3u32 {
        // The application rewrites its own array between draws; the next
        // draw must see the new bytes, not a copy kept from the first.
        bump(&mut gen, (frame as usize) * 3, VERTS, frame);
        let data = interleaved(&gen);
        attrib(0, 3, 24, 0, data.as_ptr().cast());
        attrib(1, 3, 24, 12, data.as_ptr().cast());
        let s = draw_and_check(Some(&idx), &gen, &format!("client arrays frame {frame}"));
        assert!(s.attribs.iter().all(|a| !a.1), "client arrays are staged, not fetched in place");
    }
    cleanup(fbo, tex, &[]);
}
