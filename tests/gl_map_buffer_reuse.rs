// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Buffer mapping for writing over several frames, the way glmark2's
//! `buffer` scene does it (docs/notes/imx8mp-glmark2-cpu.md, "4. Smaller"):
//! map, write a range, unmap, draw, end the frame.
//!
//! The stand-in OSAL counts the allocations, the buffer storages still alive and
//! the bytes the GL core copies from one storage to another (the source of
//! every copy is invalidated first, so the bytes of `cache_invd` are the
//! bytes copied; a write-only map invalidates nothing else). The stand-in
//! back end models the kicks itself: the kick being recorded is pending
//! from its first draw, a flush submits it, and the last `lag` submitted
//! kicks stay pending (in flight) until later flushes. lag 1 is a
//! double-buffered window, whose swap waits for all but the last kick
//! (egl.rs, `finish_previous`); lag 0 is a window rendered in place. A kick
//! reads the vertex bytes of its draws when it completes, so the test sees
//! what the GPU would have drawn, and a storage handed back too early shows
//! as wrong bytes.
//!
//! So these tests pin what the GL core's map path does with a busy or an
//! idle buffer. They do not measure the GPU back ends; the host time of
//! glmark2's scene is in the commit that added them.
//!
//! * A buffer no pending kick reads is mapped in place: no allocation and
//!   no copy at any map.
//! * glmark2's own order (`MainLoop::draw`: the scene's draw, then its
//!   update) maps a buffer the kick being recorded reads. That map moves the
//!   buffer to other pages with every byte the caller keeps (all of them
//!   for glMapBufferOES), and the draw recorded before it still draws the
//!   old bytes. The pages are a storage the buffer moved off earlier, once
//!   its kick is done: after the first `lag + 1` maps, none allocates.
//! * With `GL_MAP_INVALIDATE_RANGE_BIT` only the bytes outside the mapped
//!   range are copied; with `GL_MAP_INVALIDATE_BUFFER_BIT` none.
//! * glBufferData and glDeleteBuffers hand the kept storages back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_map_buffer_reuse
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the model: counters, kicks and what each kick drew ------------------- */

/// Bytes of the vertex buffer: 64 vertices of one vec3 (glmark2's are four
/// vec3, interleaved; the layout does not matter here).
const VERTS: usize = 64;
const BYTES: usize = VERTS * 12;

#[derive(Default)]
struct Model {
    allocs: u32,
    /// Storages of the buffer's size allocated and not handed back.
    buffer_storages: u32,
    /// Bytes `cache_invd` was asked for: the bytes copied between storages.
    invd_bytes: usize,
    /// Submitted kicks that stay pending after a flush.
    lag: u32,
    /// Kicks submitted so far; the kick being recorded is the next.
    submitted: u32,
    /// Kicks completed so far (ids 1..=completed).
    completed: u32,
    /// The kick being recorded holds a draw.
    recording: bool,
    /// The vertex storage of each draw of the kick being recorded, as its
    /// address.
    open: Vec<u64>,
    /// The same for each submitted kick that has not completed, oldest first.
    in_flight: Vec<Vec<u64>>,
    /// What each draw read when its kick completed, in draw order.
    drawn: Vec<Vec<u8>>,
}

impl Model {
    /// Complete the oldest submitted kick: its draws read their bytes now.
    fn complete_one(&mut self) {
        for gpu in self.in_flight.remove(0) {
            let mut v = vec![0u8; BYTES];
            // SAFETY: host test: the GPU address is the CPU address of a
            // storage that is never handed back (m_gpu_alloc).
            unsafe { core::ptr::copy_nonoverlapping(gpu as *const u8, v.as_mut_ptr(), BYTES) };
            self.drawn.push(v);
        }
        self.completed += 1;
    }

    /// Submit the kick being recorded, then complete all but the last `keep`.
    fn flush(&mut self, keep: u32) {
        if self.recording {
            self.in_flight.push(core::mem::take(&mut self.open));
            self.submitted += 1;
            self.recording = false;
        }
        while self.submitted - self.completed > keep {
            self.complete_one();
        }
    }
}

static MODEL: Mutex<Option<Model>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Model) -> R) -> R {
    f(MODEL.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Model::default))
}

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    /* never handed back: a freed storage stays readable, as the OSAL's
     * deferred free keeps it for the kicks that read it */
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    with(|m| {
        m.allocs += 1;
        m.buffer_storages += u32::from(size == BYTES);
    });
    0
}
unsafe extern "C" fn m_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    if !(*mem).impl_.is_null() && (*mem).size == BYTES {
        with(|m| m.buffer_storages -= 1);
    }
    0
}
unsafe extern "C" fn m_flush(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_invd(_cpu: *mut c_void, n: usize) {
    with(|m| m.invd_bytes += n);
}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
/// Record a draw into the kick being recorded: its vertex storage is read
/// when the kick completes, not now.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let a = &d.attribs[0];
    assert!(d.num_attribs == 1 && a.buffer_gpu != 0 && a.cpu_bytes == 0, "the array is fetched in place");
    with(|m| {
        m.open.push(a.buffer_gpu + a.offset as u64);
        m.recording = true;
    });
    0
}
/// Submit the kick being recorded; the last `lag` submitted stay in flight.
unsafe extern "C" fn b_flush() -> i32 {
    with(|m| m.flush(m.lag));
    0
}
/// Submit and complete every kick.
unsafe extern "C" fn b_finish() -> i32 {
    with(|m| m.flush(0));
    0
}
unsafe extern "C" fn b_kick_id() -> u32 {
    with(|m| m.submitted + 1)
}
unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    with(|m| id != 0 && id > m.completed && (id <= m.submitted || (id == m.submitted + 1 && m.recording)))
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
    cache_flush: Some(m_flush),
    cache_invd: Some(m_invd),
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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"gl-map-buffer-reuse".as_ptr() as *const c_char,
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
    flush: Some(b_flush),
    finish: Some(b_finish),
    finish_previous: None,
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context, every kick completed, and `lag` kicks kept in flight
/// from here on.
fn context(lag: u32) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with(|m| {
        m.drawn.clear();
        m.lag = lag;
    });
    g
}

/// Storages of the buffer's size allocated and not handed back (nothing
/// else these tests make is that size).
fn live() -> u32 {
    with(|m| m.buffer_storages)
}

const VS: &str = "attribute vec3 a;\nvoid main() { gl_Position = vec4(a, 1.0); }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

fn program() -> GLuint {
    let shader = |kind: GLenum, text: &str| -> GLuint {
        let text = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(kind);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    };
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"a".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link");
        p
    }
}

/// The scene's state: a program and a GL_DYNAMIC_DRAW vertex buffer filled
/// with `contents`, bound and set up as attribute 0.
struct Scene {
    program: GLuint,
    vbo: GLuint,
    /// What the buffer holds now, as the application wrote it.
    contents: Vec<u8>,
}

/// What one map cost: the allocations and the bytes copied.
type Cost = (u32, usize);

impl Scene {
    fn new() -> Self {
        let contents: Vec<u8> = (0..BYTES).map(|i| (i % 251) as u8).collect();
        let program = program();
        let mut vbo = 0;
        // SAFETY: plain GL calls; `contents` outlives glBufferData.
        unsafe {
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(GL_ARRAY_BUFFER, BYTES as GLsizeiptr, contents.as_ptr() as *const c_void, GL_DYNAMIC_DRAW);
            glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE_B, 12, core::ptr::null());
            glEnableVertexAttribArray(0);
            glUseProgram(program);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        Scene { program, vbo, contents }
    }

    fn draw(&self) {
        // SAFETY: plain GL call; the array reads the bound buffer.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, VERTS as GLsizei) };
    }

    /// glmark2's `Mesh::update_single_vbo` with update-method=map: map the
    /// whole buffer write-only (glMapBufferOES), write `[lo, hi)` with
    /// `value`, unmap.
    fn update_oes(&mut self, lo: usize, hi: usize, value: u8) -> Cost {
        let before = with(|m| (m.allocs, m.invd_bytes));
        // SAFETY: the mapping is valid for the whole store until the unmap.
        let p = unsafe { glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES) } as *mut u8;
        assert!(!p.is_null());
        let cost = with(|m| (m.allocs - before.0, m.invd_bytes - before.1));
        // SAFETY: [lo, hi) lies inside the mapped store.
        unsafe {
            core::ptr::write_bytes(p.add(lo), value, hi - lo);
            assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
        self.contents[lo..hi].fill(value);
        cost
    }

    /// As `update_oes` through glMapBufferRange of `[lo, hi)` with `access`.
    fn update_range(&mut self, lo: usize, hi: usize, access: GLbitfield, value: u8) -> Cost {
        let before = with(|m| (m.allocs, m.invd_bytes));
        // SAFETY: the mapping is valid for `hi - lo` bytes until the unmap.
        let p = unsafe {
            glMapBufferRange(GL_ARRAY_BUFFER, lo as GLintptr, (hi - lo) as GLsizeiptr, GL_MAP_WRITE_BIT | access)
        } as *mut u8;
        assert!(!p.is_null());
        let cost = with(|m| (m.allocs - before.0, m.invd_bytes - before.1));
        // SAFETY: the mapping holds `hi - lo` bytes.
        unsafe {
            core::ptr::write_bytes(p, value, hi - lo);
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
        self.contents[lo..hi].fill(value);
        cost
    }

    /// glDeleteBuffers of the scene's buffer, before the scene ends.
    fn delete_buffer(&mut self) {
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteBuffers(1, &self.vbo);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        self.vbo = 0; /* glDeleteBuffers ignores 0 (Drop) */
    }

    /// The end of a frame: eglSwapBuffers' flush.
    fn swap(&self) {
        // SAFETY: plain GL call.
        unsafe { glFlush() };
    }

    /// Run `FRAMES` frames in glmark2's order (draw, update, swap), then one
    /// more draw, and wait for every kick. Returns each update's cost and
    /// checks that every draw saw the bytes of its own frame.
    fn glmark2_frames(&mut self, mut update: impl FnMut(&mut Scene, usize) -> Cost) -> Vec<Cost> {
        let mut want = Vec::new();
        let mut costs = Vec::new();
        for f in 0..FRAMES {
            self.draw();
            want.push(self.contents.clone());
            costs.push(update(self, f));
            self.swap();
        }
        self.draw();
        want.push(self.contents.clone());
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        assert_eq!(with(|m| core::mem::take(&mut m.drawn)), want, "each draw saw the bytes of its own frame");
        costs
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        // SAFETY: plain GL calls.
        unsafe {
            glDisableVertexAttribArray(0);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDeleteBuffers(1, &self.vbo);
            glUseProgram(0);
            glDeleteProgram(self.program);
        }
    }
}

/// The frames' updates: a different range each frame, as the wave moves.
fn frame_range(f: usize) -> (usize, usize) {
    let lo = (f * 96) % (BYTES / 2);
    (lo, lo + BYTES / 2)
}

const FRAMES: usize = 8;

/// The allocations of the busy maps of `FRAMES` frames with `lag` kicks in
/// flight: the first `lag + 1` maps find no storage whose kick is done.
fn allocs_with_lag(lag: u32) -> Vec<u32> {
    (0..FRAMES as u32).map(|f| u32::from(f <= lag)).collect()
}

/// Map, write, unmap, draw, swap, with every kick done at the swap: the
/// buffer the map writes is read by no pending kick, so it is mapped in place
/// every frame.
#[test]
fn an_idle_buffer_is_mapped_in_place_at_every_frame() {
    let _g = context(0);
    let mut s = Scene::new();
    s.swap();
    let mut want = Vec::new();
    for f in 0..FRAMES {
        let (lo, hi) = frame_range(f);
        let cost = s.update_oes(lo, hi, 0x80 + f as u8);
        assert_eq!(cost, (0, 0), "frame {f}: no allocation and no copy");
        s.draw();
        want.push(s.contents.clone());
        s.swap();
    }
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
    assert_eq!(with(|m| core::mem::take(&mut m.drawn)), want, "each frame drew its own update");
}

/// glmark2's order with glMapBufferOES, the GL's own and the window's one
/// frame in flight. Every map comes while the kick being recorded reads the
/// buffer, so it copies the whole buffer (OES_mapbuffer keeps what is not
/// written), and the recorded draw still draws the old bytes. Only the first
/// maps allocate; after that the buffer cycles through the storages it moved
/// off.
#[test]
fn glmark2_order_reuses_the_storages_the_buffer_moved_off() {
    for lag in [0, 1] {
        let _g = context(lag);
        let mut s = Scene::new();
        s.swap();
        let live_before = live();
        let costs = s.glmark2_frames(|s, f| {
            let (lo, hi) = frame_range(f);
            s.update_oes(lo, hi, 0x40 + f as u8)
        });
        let allocs: Vec<u32> = costs.iter().map(|c| c.0).collect();
        assert_eq!(allocs, allocs_with_lag(lag), "lag {lag}: the allocation of each map");
        assert!(costs.iter().all(|c| c.1 == BYTES), "lag {lag}: every map copies the whole buffer: {costs:?}");
        assert_eq!(live() - live_before, lag + 1, "lag {lag}: the storages the buffer keeps");
        s.delete_buffer();
        assert_eq!(live_before - live(), 1, "lag {lag}: glDeleteBuffers hands every storage back");
    }
}

/// glMapBufferRange with GL_MAP_INVALIDATE_RANGE_BIT on a busy buffer: the
/// mapped range is given up, so only the bytes outside it are copied, and
/// those keep their contents. Into fresh pages that is every byte outside
/// the range; into a retired storage only the ranges mapped since it was
/// retired, outside this one (tests/gl_buffer_retired_reuse.rs).
#[test]
fn invalidate_range_copies_only_the_bytes_kept() {
    let _g = context(1);
    let mut s = Scene::new();
    s.swap();
    let costs = s.glmark2_frames(|s, f| {
        let (lo, hi) = frame_range(f);
        s.update_range(lo, hi, GL_MAP_INVALIDATE_RANGE_BIT, 0x20 + f as u8)
    });
    let allocs: Vec<u32> = costs.iter().map(|c| c.0).collect();
    assert_eq!(allocs, allocs_with_lag(1));
    for (f, c) in costs.iter().enumerate() {
        if c.0 == 1 {
            assert_eq!(c.1, BYTES / 2, "frame {f}: fresh pages get every byte outside the mapped half");
        } else {
            assert!(c.1 < BYTES / 2, "frame {f}: a retired storage gets the dirty ranges only: {c:?}");
        }
    }
}

/// GL_MAP_INVALIDATE_BUFFER_BIT on a busy buffer: nothing is copied. The
/// whole store is written, so what the draw after it sees is defined.
#[test]
fn invalidate_buffer_copies_nothing() {
    let _g = context(1);
    let mut s = Scene::new();
    s.swap();
    let costs = s.glmark2_frames(|s, f| s.update_range(0, BYTES, GL_MAP_INVALIDATE_BUFFER_BIT, 0x10 + f as u8));
    let want: Vec<Cost> = allocs_with_lag(1).into_iter().map(|a| (a, 0)).collect();
    assert_eq!(costs, want);
}

/// glBufferData gives the store a new life: the storages kept for maps of the
/// old one are handed back, and the next busy map allocates again.
#[test]
fn buffer_data_hands_the_kept_storages_back() {
    let _g = context(1);
    let mut s = Scene::new();
    s.swap();
    let live_before = live();
    s.glmark2_frames(|s, f| {
        let (lo, hi) = frame_range(f);
        s.update_oes(lo, hi, 0x60 + f as u8)
    });
    assert_eq!(live() - live_before, 2, "the storages the buffer keeps");
    // SAFETY: plain GL calls; `contents` outlives glBufferData.
    unsafe {
        glBufferData(GL_ARRAY_BUFFER, BYTES as GLsizeiptr, s.contents.as_ptr() as *const c_void, GL_DYNAMIC_DRAW);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    assert_eq!(live(), live_before, "only the buffer's own storage is left");
    s.draw();
    let (lo, hi) = frame_range(0);
    assert_eq!(s.update_oes(lo, hi, 0x70).0, 1, "nothing kept: the busy map allocates");
}
