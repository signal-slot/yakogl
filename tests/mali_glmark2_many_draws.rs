// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of glmark2's ideas scene shape on the Mali back end:
//! many small `glDrawElements` calls in one frame, a `glUniformMatrix4fv`
//! translation and a `glUniform4f` colour between draws, two programs
//! alternating every 16 draws, `GL_UNSIGNED_SHORT` indices from an IBO. The
//! shaders are written for this test; no glmark2 text is copied.
//!
//! Level reached: the draws handed to the back end. The host has no GPU, so
//! no pixel is written and the "read back" is of what the back end is handed
//! per draw (the stand-in back end of `tests/mali_glmark2_scene_draw.rs`
//! records each `GlesDraw`): the vertex uniform block must hold that draw's
//! matrix, the fragment block that draw's colour, the index range must be
//! that draw's quad, and the program must be the one for its group of 16.
//! A value left over from an earlier draw would fail one of these. 3 frames
//! run; each must raise no GL error, and the GPU allocations the library
//! makes must not grow from frame 2 to frame 3 (the mock's only counter).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_many_draws
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    ALLOCS.fetch_add(1, Ordering::Relaxed);
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
    vs: usize,
    index_count: u32,
    index_size: u8,
    vs_uniforms: Vec<u32>,
    fs_uniforms: Vec<u32>,
    indices: Vec<u16>,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe fn words(p: *const c_void, n: u16) -> Vec<u32> {
    if p.is_null() {
        return Vec::new();
    }
    core::slice::from_raw_parts(p.cast::<u32>(), n as usize).to_vec()
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    let mali = mali_draw(d);
    let indices = if d.index_buf_gpu != 0 {
        // the stand-in's allocations are their own addresses
        core::slice::from_raw_parts(d.index_buf_gpu as *const u16, d.index_count as usize).to_vec()
    } else if !d.index_cpu.is_null() {
        core::slice::from_raw_parts(d.index_cpu.cast::<u16>(), d.index_count as usize).to_vec()
    } else {
        Vec::new()
    };
    SEEN.lock().unwrap().push(Seen {
        vs: d.vs as usize,
        index_count: d.index_count,
        index_size: d.index_size,
        vs_uniforms: words(d.vs_uniform_cpu, d.vs_uniform_dwords),
        fs_uniforms: words(d.fs_uniform_cpu, d.fs_uniform_dwords),
        indices,
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
        p
    }
}

/// Program A: a matrix and a colour. Program B: the same inputs, the colour
/// scaled and offset (so the two fragment blocks differ in shape of use).
const VS: &str = "attribute vec3 a_pos;
uniform mat4 u_mvp;
void main()
{
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
";
const FS_A: &str = "precision mediump float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color;
}
";
const FS_B: &str = "precision mediump float;
uniform vec4 u_color;
void main()
{
    gl_FragColor = u_color * 0.5 + vec4(0.25, 0.25, 0.25, 0.0);
}
";

const QUADS: usize = 256;
const GRID: usize = 16;

/// The quad `i`'s translation (a 16 x 16 grid over clip space) and colour.
fn place(i: usize) -> ([f32; 16], [f32; 4]) {
    let (cx, cy) = ((i % GRID) as f32, (i / GRID) as f32);
    let (tx, ty) = (-1.0 + (cx + 0.5) * 2.0 / GRID as f32, -1.0 + (cy + 0.5) * 2.0 / GRID as f32);
    let mut m = [0.0f32; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    m[12] = tx;
    m[13] = ty;
    (m, [(i % 7) as f32 / 8.0 + 0.125, (i % 11) as f32 / 16.0 + 0.0625, i as f32 / 512.0, 1.0 - i as f32 / 1024.0])
}

fn find_run(hay: &[u32], needle: &[f32]) -> Option<usize> {
    let n: Vec<u32> = needle.iter().map(|f| f.to_bits()).collect();
    hay.windows(n.len()).position(|w| w == &n[..])
}

#[test]
fn many_small_draws_keep_their_own_uniforms() {
    let _g = context();
    let (w, h) = (64, 64);
    let (mut fbo, mut tex, mut vbo, mut ibo) = (0, 0, 0, 0);
    // quad `i` owns vertices 4i..4i+4 (a 1/16-wide square at the origin) and
    // the six u16 indices at 6i.
    let mut verts = Vec::new();
    let mut idx: Vec<u16> = Vec::new();
    let q = 1.0 / GRID as f32 * 0.8;
    for i in 0..QUADS {
        for (x, y) in [(-q, -q), (q, -q), (-q, q), (q, q)] {
            verts.extend_from_slice(&[x, y, 0.0]);
        }
        let b = (i * 4) as u16;
        idx.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    let pa = program(VS, FS_A);
    let pb = program(VS, FS_B);
    // SAFETY: room for the names; the arrays outlive the calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, (verts.len() * 4) as isize, verts.as_ptr().cast(), GL_STATIC_DRAW);
        glGenBuffers(1, &mut ibo);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
        glBufferData(GL_ELEMENT_ARRAY_BUFFER, (idx.len() * 2) as isize, idx.as_ptr().cast(), GL_STATIC_DRAW);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(0);
    }
    assert_eq!(err(), GL_NO_ERROR, "set-up");

    let mut allocs = Vec::new();
    for frame in 0..3 {
        SEEN.lock().unwrap().clear();
        // SAFETY: the programs and buffers above; uniform arrays are local.
        unsafe {
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            let mut cur = 0;
            for i in 0..QUADS {
                let p = if (i / 16) % 2 == 0 { pa } else { pb };
                if p != cur {
                    glUseProgram(p);
                    cur = p;
                }
                let (m, c) = place(i);
                glUniformMatrix4fv(glGetUniformLocation(p, c"u_mvp".as_ptr()), 1, 0, m.as_ptr());
                glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), c[0], c[1], c[2], c[3]);
                glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (i * 12) as *const c_void);
            }
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR, "frame {frame}");
        let seen = core::mem::take(&mut *SEEN.lock().unwrap());
        assert_eq!(seen.len(), QUADS, "frame {frame}: every draw reached the back end");
        let (mut va, mut vb) = (None, None);
        for (i, s) in seen.iter().enumerate() {
            let (m, c) = place(i);
            let what = format!("frame {frame} draw {i}");
            let group = (i / 16) % 2;
            let slot = if group == 0 { &mut va } else { &mut vb };
            // the program of the group: one shader per program, never mixed
            assert_eq!(*slot.get_or_insert(s.vs), s.vs, "{what}: program");
            assert_eq!((s.index_count, s.index_size), (6, 2), "{what}");
            let b = (i * 4) as u16;
            assert_eq!(s.indices, [b, b + 1, b + 2, b + 2, b + 1, b + 3], "{what}: its own quad's indices");
            assert!(find_run(&s.vs_uniforms, &m).is_some(), "{what}: its own matrix in {:x?}", s.vs_uniforms);
            // a colour of another draw must not be in the block
            assert!(find_run(&s.fs_uniforms, &c).is_some(), "{what}: its own colour in {:x?}", s.fs_uniforms);
            if i > 0 {
                let (pm, pc) = place(i - 1);
                assert!(find_run(&s.vs_uniforms, &pm).is_none(), "{what}: the previous matrix leaked");
                assert!(find_run(&s.fs_uniforms, &pc).is_none(), "{what}: the previous colour leaked");
            }
            assert_eq!(s.mali, -yakogl::util::ENODEV, "{what}: the Mali takes the draw");
        }
        assert_ne!(va.unwrap(), vb.unwrap(), "the two programs are two shaders");
        allocs.push(ALLOCS.load(Ordering::Relaxed));
    }
    assert_eq!(allocs[1], allocs[2], "GPU allocations grew from frame 2 to 3: {allocs:?}");
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glDeleteBuffers(1, &vbo);
        glDeleteBuffers(1, &ibo);
        glDeleteProgram(pa);
        glDeleteProgram(pb);
    }
    assert_eq!(err(), GL_NO_ERROR, "clean-up");
}
