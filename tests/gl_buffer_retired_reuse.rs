// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A write to a buffer a pending kick reads, the way glmark2's `buffer`
//! scene makes it (docs/notes/gl-buffer-update.md, "Ranked fixes" 1 and 2):
//! the draw of the frame first, then about ten glBufferSubData calls into
//! the buffer it reads, then the swap.
//!
//! The stand-in OSAL and back end are tests/gl_map_buffer_reuse.rs's: the
//! OSAL counts the allocations and the bytes the GL core copies from one
//! storage to another (the source of every copy is invalidated first, so
//! the bytes of `cache_invd` are the bytes copied); the back end keeps the
//! last `lag` submitted kicks in flight, and a kick reads the vertex bytes of
//! its draws only when it completes, so a write that reached a storage a
//! kick still reads shows as wrong bytes. This OSAL also hands a freed
//! storage out again once the kicks that may read it are done, cleared, as
//! the Linux OSAL's deferred free and memset do: the timing at the end then
//! pays what the host harness pays for an allocation, not a page fault on
//! fresh memory.
//!
//! * The first write of a frame moves the buffer off the storage the kick
//!   reads. After the first `lag + 1` frames it moves to a storage it moved
//!   off before, whose kick is done: no allocation, at glBufferSubData as at
//!   a map.
//! * That storage lacks only the ranges written since it was retired, and
//!   only those are copied, not the whole buffer.
//! * Every draw draws the bytes of its own frame, equal to a reference copy
//!   the test keeps, with subdata, maps, copies and a mix of them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_buffer_retired_reuse
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gl_buffer_retired_reuse
//! # the timing of glmark2's update calls (docs/notes/gl-buffer-update.md, "Done"):
//! cargo test --release --no-default-features --features host-test,gpu-powervr,shaderc-online \
//!   --test gl_buffer_retired_reuse -- --ignored --nocapture
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

/// Bytes of the vertex buffer: 256 vertices of one vec3.
const VERTS: usize = 256;
const BYTES: usize = VERTS * 12;

#[derive(Default)]
struct Model {
    allocs: u32,
    /// Freed storages, `(size, address, kick)`: reusable once kick `kick`
    /// (the one being recorded at the free) has completed.
    freed: Vec<(usize, u64, u32)>,
    /// Bytes `cache_invd` was asked for: the bytes copied between storages.
    invd_bytes: usize,
    lag: u32,
    submitted: u32,
    completed: u32,
    recording: bool,
    open: Vec<u64>,
    in_flight: Vec<Vec<u64>>,
    drawn: Vec<Vec<u8>>,
}

impl Model {
    fn complete_one(&mut self) {
        for gpu in self.in_flight.remove(0) {
            let mut v = vec![0u8; BYTES];
            // SAFETY: host test: the GPU address is the CPU address of a
            // storage that is never released, only reused once the kicks
            // that may read it are done (m_gpu_free).
            unsafe { core::ptr::copy_nonoverlapping(gpu as *const u8, v.as_mut_ptr(), BYTES) };
            self.drawn.push(v);
        }
        self.completed += 1;
    }

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
    /* A freed storage of the size whose kicks are done comes back (the
     * OSAL's deferred free: a storage handed back too early would be
     * overwritten here and show as wrong bytes at a draw), else new memory,
     * never released. Cleared with a memset either way, as
     * src/osal/linux.c's l_gpu_alloc clears. */
    let reuse = with(|m| {
        m.allocs += 1;
        let done = m.completed;
        let at = m.freed.iter().position(|&(s, _, k)| s == size && k <= done)?;
        Some(m.freed.swap_remove(at).1 as *mut u8)
    });
    let p = reuse.unwrap_or_else(|| Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr());
    core::ptr::write_bytes(p, 0, size);
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    let m_ = &*mem;
    if !m_.impl_.is_null() {
        with(|m| {
            let kick = m.submitted + 1; /* the kick being recorded */
            m.freed.push((m_.size, m_.gpu, kick));
        });
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
unsafe extern "C" fn b_flush() -> i32 {
    with(|m| m.flush(m.lag));
    0
}
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
    name: c"gl-buffer-retired-reuse".as_ptr() as *const c_char,
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

/// What one frame's update cost: the allocations and the bytes copied
/// between storages.
type Cost = (u32, usize);

/// The scene: a program and a GL_DYNAMIC_DRAW vertex buffer, bound and set
/// up as attribute 0, with the reference copy of what it holds.
struct Scene {
    program: GLuint,
    vbo: GLuint,
    /// What the buffer holds now, as the application wrote it.
    contents: Vec<u8>,
}

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

    /// glBufferSubData of `[lo, hi)` with bytes made from `seed`.
    fn sub_data(&mut self, lo: usize, hi: usize, seed: u8) {
        let bytes: Vec<u8> = (lo..hi).map(|i| seed.wrapping_add((i * 7) as u8)).collect();
        // SAFETY: plain GL call; `bytes` holds `hi - lo` bytes.
        unsafe {
            glBufferSubData(GL_ARRAY_BUFFER, lo as GLintptr, (hi - lo) as GLsizeiptr, bytes.as_ptr() as *const c_void);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        self.contents[lo..hi].copy_from_slice(&bytes);
    }

    /// glMapBufferOES of the whole store, `[lo, hi)` written with `value`.
    fn map_oes(&mut self, lo: usize, hi: usize, value: u8) {
        // SAFETY: the mapping is valid for the whole store until the unmap.
        unsafe {
            let p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES) as *mut u8;
            assert!(!p.is_null());
            core::ptr::write_bytes(p.add(lo), value, hi - lo);
            assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
        self.contents[lo..hi].fill(value);
    }

    /// glMapBufferRange of `[lo, hi)` with `access` (and WRITE), written
    /// with `value`.
    fn map_range(&mut self, lo: usize, hi: usize, access: GLbitfield, value: u8) {
        // SAFETY: the mapping holds `hi - lo` bytes until the unmap.
        unsafe {
            let p =
                glMapBufferRange(GL_ARRAY_BUFFER, lo as GLintptr, (hi - lo) as GLsizeiptr, GL_MAP_WRITE_BIT | access)
                    as *mut u8;
            assert!(!p.is_null());
            core::ptr::write_bytes(p, value, hi - lo);
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
        self.contents[lo..hi].fill(value);
    }

    /// glCopyBufferSubData of `[from, from + n)` of the buffer to `to` (the
    /// ranges must not overlap), through the ES 3.0 copy targets.
    fn copy_within(&mut self, from: usize, to: usize, n: usize) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_COPY_READ_BUFFER, self.vbo);
            glBindBuffer(GL_COPY_WRITE_BUFFER, self.vbo);
            glCopyBufferSubData(
                GL_COPY_READ_BUFFER,
                GL_COPY_WRITE_BUFFER,
                from as GLintptr,
                to as GLintptr,
                n as GLsizeiptr,
            );
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        self.contents.copy_within(from..from + n, to);
    }

    /// The end of a frame: eglSwapBuffers' flush.
    fn swap(&self) {
        // SAFETY: plain GL call.
        unsafe { glFlush() };
    }

    /// Run `frames` frames in glmark2's order (draw, update, swap), then one
    /// more draw, and wait for every kick. Returns each update's cost and
    /// checks that every draw saw the bytes of its own frame.
    fn glmark2_frames(&mut self, frames: usize, mut update: impl FnMut(&mut Scene, usize)) -> Vec<Cost> {
        let mut want = Vec::new();
        let mut costs = Vec::new();
        for f in 0..frames {
            self.draw();
            want.push(self.contents.clone());
            let before = with(|m| (m.allocs, m.invd_bytes));
            update(self, f);
            costs.push(with(|m| (m.allocs - before.0, m.invd_bytes - before.1)));
            self.swap();
        }
        self.draw();
        want.push(self.contents.clone());
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        let drawn = with(|m| core::mem::take(&mut m.drawn));
        assert_eq!(drawn.len(), want.len(), "one read a draw");
        for (f, (d, w)) in drawn.iter().zip(&want).enumerate() {
            assert!(d == w, "draw {f} saw other bytes than those of its own frame (first at {:?})", first_diff(d, w));
        }
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

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// glmark2's subdata update: about ten ranges of a buffer, the same ones
/// every frame (update-fraction 0.5, update-dispersion 0.9): ten ranges of
/// 144 bytes, 304 bytes apart.
const RANGES: [(usize, usize); 10] = {
    let mut r = [(0, 0); 10];
    let mut i = 0;
    while i < 10 {
        r[i] = (i * 304, i * 304 + 144);
        i += 1;
    }
    r
};
const RANGE_BYTES: usize = 10 * 144;

/// The allocations of `frames` frames whose first write each finds the
/// buffer busy, with `lag` kicks in flight: the first `lag + 1` find no
/// retired storage whose kick is done.
fn allocs_with_lag(frames: usize, lag: u32) -> Vec<u32> {
    (0..frames as u32).map(|f| u32::from(f <= lag)).collect()
}

/// A tiny deterministic generator for the mixed test.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

/// glmark2's subdata frames: after the warm-up no glBufferSubData
/// allocates, and the first write of a frame copies only the ranges written
/// since the storage it takes back was retired (the same ten ranges), not
/// the whole buffer.
#[test]
fn sub_data_to_a_busy_buffer_reuses_retired_storage_and_copies_the_dirty_ranges() {
    for lag in [0, 1] {
        let _g = context(lag);
        let mut s = Scene::new();
        s.swap();
        let frames = 12;
        let costs = s.glmark2_frames(frames, |s, f| {
            for (k, &(lo, hi)) in RANGES.iter().enumerate() {
                s.sub_data(lo, hi, (f * 16 + k) as u8);
            }
        });
        let allocs: Vec<u32> = costs.iter().map(|c| c.0).collect();
        assert_eq!(allocs, allocs_with_lag(frames, lag), "lag {lag}: gpu_alloc calls a frame");
        for (f, c) in costs.iter().enumerate() {
            let want = if c.0 == 1 { BYTES } else { RANGE_BYTES };
            assert_eq!(c.1, want, "lag {lag} frame {f}: bytes copied between storages");
        }
    }
}

/// Subdata and maps by turns (OES, a range, a range with
/// INVALIDATE_RANGE): what each draw sees is the reference copy, and after
/// the warm-up nothing allocates.
#[test]
fn alternating_sub_data_and_maps_draw_the_reference_bytes() {
    for lag in [0, 1] {
        let _g = context(lag);
        let mut s = Scene::new();
        s.swap();
        let frames = 16;
        let costs = s.glmark2_frames(frames, |s, f| match f % 4 {
            0 => {
                for &(lo, hi) in &RANGES[..5] {
                    s.sub_data(lo, hi, f as u8);
                }
            }
            1 => s.map_oes(100, 900, 0x40 + f as u8),
            2 => {
                s.map_range(1000, 1600, 0, 0x50 + f as u8);
                s.sub_data(2000, 2100, f as u8);
            }
            _ => s.map_range(200, 2600, GL_MAP_INVALIDATE_RANGE_BIT, 0x60 + f as u8),
        });
        let allocs: Vec<u32> = costs.iter().map(|c| c.0).collect();
        assert_eq!(allocs, allocs_with_lag(frames, lag), "lag {lag}: gpu_alloc calls a frame");
    }
}

/// A full update (every byte by subdata), then partial ones: the retired
/// storages, written whole once, still take back only what they lack.
#[test]
fn a_partial_update_after_a_full_one() {
    let _g = context(1);
    let mut s = Scene::new();
    s.swap();
    let costs = s.glmark2_frames(8, |s, f| {
        if f == 2 {
            s.sub_data(0, BYTES, 0x90);
        } else {
            s.sub_data(512, 640, f as u8);
        }
    });
    let allocs: Vec<u32> = costs.iter().map(|c| c.0).collect();
    assert_eq!(allocs, allocs_with_lag(8, 1));
    /* frame 3 takes back frame 1's storage, which lacks frame 2's full
     * write; frame 4 the storage frame 2 moved off, which lacks frames 2 and 3 */
    assert_eq!(costs[3].1, BYTES, "{costs:?}");
    assert_eq!(costs[4].1, BYTES, "{costs:?}");
    assert_eq!(costs[5].1, 128, "frames 3 and 4 wrote the same 128 bytes: {costs:?}");
    assert_eq!(costs[7].1, 128, "{costs:?}");
}

/// More ranges a frame than a dirty list keeps apart (40 single bytes):
/// the list merges its closest ranges, copies more than was written, and
/// every draw still sees the reference bytes.
#[test]
fn more_ranges_than_the_dirty_list_keeps_still_draw_the_reference_bytes() {
    let _g = context(1);
    let mut s = Scene::new();
    s.swap();
    let costs = s.glmark2_frames(8, |s, f| {
        for k in 0..40 {
            let at = k * 75 + f;
            s.sub_data(at, at + 1, (f + k) as u8);
        }
    });
    let allocs: Vec<u32> = costs.iter().map(|c| c.0).collect();
    assert_eq!(allocs, allocs_with_lag(8, 1));
    assert!(costs[4..].iter().all(|c| c.1 < BYTES), "the merged list still leaves gaps out: {costs:?}");
}

/// Random writes of every kind, a random number a frame (none some frames,
/// so a frame's writes find the buffer idle), with 0 to 2 kicks in flight:
/// every draw sees the reference bytes.
#[test]
fn random_writes_of_every_kind_draw_the_reference_bytes() {
    for lag in [0, 1, 2] {
        let _g = context(lag);
        let mut s = Scene::new();
        s.swap();
        let mut rng = Lcg(0x5eed + u64::from(lag));
        s.glmark2_frames(40, |s, _| {
            for _ in 0..rng.next(5) {
                let lo = rng.next(BYTES - 1);
                let hi = lo + 1 + rng.next((BYTES - lo).min(700));
                let v = rng.next(256) as u8;
                match rng.next(5) {
                    0 | 1 => s.sub_data(lo, hi, v),
                    2 => s.map_oes(lo, hi, v),
                    3 => s.map_range(lo, hi, if rng.next(2) == 0 { 0 } else { GL_MAP_INVALIDATE_RANGE_BIT }, v),
                    _ if !yakogl::REPORT_ES2 => {
                        let n = 1 + rng.next(300);
                        let (from, to) = (rng.next(BYTES / 2 - n), BYTES / 2 + rng.next(BYTES / 2 - n));
                        s.copy_within(from, to, n);
                    }
                    _ => s.sub_data(lo, hi, v),
                }
            }
        });
    }
}

/// A write to the buffer while no kick reads it lands in place, and still
/// reaches the retired storages' dirty lists: the next busy write that takes
/// one back carries it.
#[test]
fn an_idle_write_between_busy_ones_is_carried() {
    let _g = context(0);
    let mut s = Scene::new();
    s.swap();
    s.glmark2_frames(6, |s, f| {
        s.sub_data(0, 64, f as u8);
        // SAFETY: plain GL call: the swap completes the frame's kick.
        unsafe { glFinish() };
        s.sub_data(1000 + f * 100, 1050 + f * 100, 0xA0 + f as u8);
    });
}

/* ---- a timing, not a check ------------------------------------------------- */

/// glmark2's buffer scene at its own sizes, in its own order, with the
/// window's one frame in flight: four attribute buffers of 24,000 vec3
/// (288,000 bytes), each drawn and then updated by `method`: 10
/// glBufferSubData ranges of 14,400 bytes (half the buffer,
/// update-fraction 0.5), or one glMapBufferOES with the same 10 ranges
/// written. Returns the wall time of the update calls a frame in ms (the
/// median of frames 20 to 219) and the gpu_alloc calls of those frames.
fn time_glmark2_updates(map: bool) -> (f64, u32) {
    const BIG: usize = 288_000;
    const PIECE: usize = BIG / 20;
    let _g = context(1);
    let program = program();
    let data = vec![0x11u8; BIG];
    let piece = vec![0x22u8; PIECE];
    let mut vbos = [0u32; 4];
    // SAFETY: plain GL calls; `data` outlives glBufferData.
    unsafe {
        glUseProgram(program);
        glGenBuffers(4, vbos.as_mut_ptr());
        for &v in &vbos {
            glBindBuffer(GL_ARRAY_BUFFER, v);
            glBufferData(GL_ARRAY_BUFFER, BIG as GLsizeiptr, data.as_ptr() as *const c_void, GL_DYNAMIC_DRAW);
        }
        glEnableVertexAttribArray(0);
    }
    let (mut ms, mut allocs) = (Vec::new(), 0);
    for f in 0..220 {
        // SAFETY: plain GL calls; each mapping holds BIG bytes until its unmap.
        unsafe {
            for &v in &vbos {
                glBindBuffer(GL_ARRAY_BUFFER, v);
                glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE_B, 12, core::ptr::null());
                glDrawArrays(GL_TRIANGLES, 0, VERTS as GLsizei);
            }
            let a0 = with(|m| m.allocs);
            let t = std::time::Instant::now();
            for &v in &vbos {
                glBindBuffer(GL_ARRAY_BUFFER, v);
                if map {
                    let p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES) as *mut u8;
                    for k in 0..10 {
                        core::ptr::copy_nonoverlapping(piece.as_ptr(), p.add(k * 2 * PIECE), PIECE);
                    }
                    glUnmapBufferOES(GL_ARRAY_BUFFER);
                } else {
                    for k in 0..10 {
                        let at = (k * 2 * PIECE) as GLintptr;
                        glBufferSubData(GL_ARRAY_BUFFER, at, PIECE as GLsizeiptr, piece.as_ptr() as *const c_void);
                    }
                }
            }
            let dt = t.elapsed().as_secs_f64() * 1e3;
            if f >= 20 {
                ms.push(dt);
                allocs += with(|m| m.allocs) - a0;
            }
            glFlush();
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
    }
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        glDisableVertexAttribArray(0);
        glDeleteBuffers(4, vbos.as_ptr());
        glUseProgram(0);
        glDeleteProgram(program);
    }
    with(|m| m.drawn.clear());
    ms.sort_by(f64::total_cmp);
    (ms[ms.len() / 2], allocs)
}

/// `cargo test --release ... --test gl_buffer_retired_reuse -- --ignored --nocapture`
#[test]
#[ignore = "a timing, not a check: run it with --ignored --nocapture"]
fn time_glmark2_buffer_scene_updates() {
    for map in [false, true] {
        let (ms, allocs) = time_glmark2_updates(map);
        let what = if map { "glMapBufferOES + 10 ranges + unmap" } else { "10 x glBufferSubData" };
        println!("glmark2 buffer scene, 4 buffers a frame, {what}: {ms:.4} ms a frame (median), {allocs} gpu_alloc in 200 frames");
    }
}
