// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a mixed-divisor glDrawArraysInstanced with `first != 0`
//! (ES 3.0 2.9.3: an array with a divisor is fetched at instance / divisor,
//! whatever `first` is). `build_vertex_input` and `emit_streams` in
//! src/vivante/backend.rs program one stream an attribute: the base address
//! (`nfe_vertex_streams_base_addr`) is the array's buffer address plus its
//! offset, the divisor word is 0 or the divisor, and `first` goes only to the
//! third word of the DRAW_INSTANCED (the start vertex); the GL core folds it
//! into the per-vertex stream's base (a divisor-0 array) and passes start 0,
//! and it never reaches a per-instance stream's base.
//!
//! glDrawArraysInstanced(first = 5, count = 3, instances = 4) with arrays at
//! divisors 0, 1 and 2 (words 0, 1, 2 at 0x14680, 0x14684, 0x14688): the same
//! words and the same per-instance stream bases as the draw at first = 0, the
//! per-vertex base 5 vertices on, and ONE DRAW_INSTANCED of 4 instances.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_divisor_first
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArraysInstanced;
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glFinish, glGetError, glViewport};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribDivisor, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end, the state cache and
/// `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

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
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs) */
        match p.allocs.remove(&m.gpu) {
            Some(a) => core::mem::forget(a.mem),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept and its fence raised; every kick completes at once.
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
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let words = words.to_vec();
        p.kicks.push(words);
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

/* ---- GL plumbing ------------------------------------------------------------------- */

/// Run `case` on the real Vivante back end over the stand-in OSAL and a
/// fresh 64x64 context. The stand-in starts empty and hands out the same
/// addresses as at the last start (the memory of earlier cases is leaked,
/// not reused), so two runs of one GL sequence get the same streams.
fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| core::mem::take(&mut p.faults));
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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

/// A linked program of `vs` and `fs`, attribute `p` at 0, `q` at 1 and `r` at 2.
fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glBindAttribLocation(p, 1, c"q".as_ptr());
        glBindAttribLocation(p, 2, c"r".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// The streams submitted since the last call.
fn take_kicks() -> Vec<Vec<u32>> {
    with_platform(|p| core::mem::take(&mut p.kicks))
}

const VS3: &str = "attribute vec4 p;\nattribute vec4 q;\nattribute vec4 r;\nvarying vec4 v;\nvoid main() { v = q + r; gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";

/// Attribute 0: 8 vertices of 4 floats (first 5 + count 3 reach vertex 7),
/// at byte 0. Attribute 1: 4 elements of 4 floats at byte 128. Attribute 2: 2
/// elements of 4 floats at byte 192 (4 instances at divisor 2).
const OFF1: usize = 128;
const OFF2: usize = 192;
static DATA: [f32; 56] = [0.25; 56];

/// The program, a buffer of [`DATA`] and the three arrays at divisors 0, 1, 2.
fn setup() {
    // SAFETY: plain GL calls; DATA is a static.
    unsafe {
        glUseProgram(program(VS3, FS));
        glViewport(0, 0, 64, 64);
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&DATA) as GLsizeiptr,
            DATA.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, OFF1 as *const c_void);
        glEnableVertexAttribArray(2);
        glVertexAttribPointer(2, 4, GL_FLOAT, 0, 0, OFF2 as *const c_void);
        glVertexAttribDivisor(1, 1);
        glVertexAttribDivisor(2, 2);
    }
    no_error("the setup");
}

/// What the stream has loaded at a DRAW_INSTANCED.
#[derive(Debug, PartialEq)]
struct Seen {
    /// Divisor words of streams 0..3 (0x14680 + 4 i), u32::MAX: never written.
    divisor: [u32; 3],
    /// Base addresses of streams 0..3.
    base: [u32; 3],
    instances: u32,
    start: u32,
}

fn draws(kicks: &[Vec<u32>]) -> Vec<Seen> {
    let div = [0, 1, 2].map(nfe_vertex_streams_vertex_divisor);
    let base = [0, 1, 2].map(nfe_vertex_streams_base_addr);
    let mut out = Vec::new();
    for words in kicks {
        let (mut d, mut b) = ([u32::MAX; 3], [u32::MAX; 3]);
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let at = (w & 0xffff) * 4;
                    for k in 0..count {
                        let r = at + 4 * k as u32;
                        if let Some(s) = div.iter().position(|&x| x == r) {
                            d[s] = words[i + 1 + k];
                        }
                        if let Some(s) = base.iter().position(|&x| x == r) {
                            b[s] = words[i + 1 + k];
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    out.push(Seen { divisor: d, base: b, instances: w & 0xffff, start: words[i + 2] });
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    out
}

fn draw_at(first: GLint) -> Seen {
    setup();
    let _ = take_kicks();
    // SAFETY: plain GL calls on the bound arrays.
    unsafe {
        glDrawArraysInstanced(GL_TRIANGLES, first, 3, 4);
        glFinish();
    }
    no_error("glDrawArraysInstanced");
    let mut v = draws(&take_kicks());
    assert_eq!(v.len(), 1, "ONE DRAW_INSTANCED for the 4 instances: {v:?}");
    v.pop().unwrap()
}

#[test]
fn first_moves_only_the_per_vertex_stream_not_the_per_instance_bases() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let at0 = with_backend(|| draw_at(0));
    let at5 = with_backend(|| draw_at(5));

    for (name, s) in [("first 0", &at0), ("first 5", &at5)] {
        assert_eq!(s.divisor, [0, 1, 2], "{name}: words at 0x14680, 0x14684, 0x14688");
        assert_eq!(s.instances, 4, "{name}: ONE instanced draw of the 4 instances");
    }
    /* the GL core folds `first` into the per-vertex stream's base and
     * starts the draw at vertex 0 (the Vivante back end gets `first` 0 for a
     * non-indexed draw) */
    assert_eq!((at0.start, at5.start), (0, 0), "the DRAW_INSTANCED start vertex");
    assert_eq!(at5.base[0] - at0.base[0], 5 * 16, "stream 0 (divisor 0) moves by first * stride");
    assert_eq!(at5.base[1..], at0.base[1..], "first must not move a per-instance stream base");
    let b = at5.base;
    assert_eq!(b[1] - at0.base[0], OFF1 as u32, "stream 1 at the array's offset, no first * stride");
    assert_eq!(b[2] - at0.base[0], OFF2 as u32, "stream 2 at the array's offset, no first * stride");
}
