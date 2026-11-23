// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a per-vertex and a per-instance attribute interleaved in
//! ONE buffer (stride 32; attribute 0 at offset 0, divisor 0; attribute 1 at
//! offset 16, divisor 2). A GC vertex stream has one base address, one stride
//! and one divisor word, so attributes that differ in divisor must not share
//! a stream.
//!
//! What the back end does (`build_vertex_input` in src/vivante/backend.rs):
//! one stream per attribute, never merged, even when buffer and stride are
//! the same. So the tests pin:
//!
//! * the two streams' divisor words are 0 and 2 (0x14680, 0x14684), their
//!   control words carry stride 32, and stream 1's base is stream 0's + 16
//!   (the attribute offset is folded into the base);
//! * the vertex elements (`nfe_generic_attrib_config0`, 0x17800 range) name
//!   stream 0 and stream 1 and start at 0 within their stream;
//! * the 4 instances are ONE DRAW_INSTANCED;
//! * control: both attributes at divisor 0 are still two streams (pinned: not
//!   merged), both words 0.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_divisor_shared
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

/* ---- the cases --------------------------------------------------------------------- */

const VS: &str = "attribute vec4 p;\nattribute vec4 q;\nvarying vec4 v;\n\
                  void main() { v = q; gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";

/// Four interleaved records of 32 bytes: a position (offset 0) and a colour
/// (offset 16), eight floats each.
static DATA: [f32; 32] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, //
    1.0, -1.0, 0.5, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

/// The program and one VBO with both attributes in it (stride 32, offsets 0
/// and 16), bound and enabled; attribute 1 at `divisor`.
fn setup(divisor: GLuint) {
    // SAFETY: plain GL calls; DATA is a static.
    unsafe {
        glUseProgram(program(VS, FS));
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
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 32, core::ptr::null());
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 32, 16 as *const c_void);
        glVertexAttribDivisor(1, divisor);
    }
    no_error("the setup");
}

/// The front-end state as loaded by the end of a kick set.
#[derive(Default, Debug)]
struct Fe {
    base: [u32; 2],
    control: [u32; 2],
    divisor: [u32; 2],
    cfg0: [u32; 2],
    cfg1: [u32; 2],
    /// the instance counts of every DRAW_INSTANCED
    counts: Vec<u32>,
}

fn front_end(kicks: &[Vec<u32>]) -> Fe {
    let mut fe = Fe { base: [u32::MAX; 2], control: [u32::MAX; 2], divisor: [u32::MAX; 2], ..Fe::default() };
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let r = base + 4 * k as u32;
                        let v = words[i + 1 + k];
                        for s in 0..2u32 {
                            let u = s as usize;
                            if r == nfe_vertex_streams_base_addr(s) {
                                fe.base[u] = v;
                            } else if r == nfe_vertex_streams_control(s) {
                                fe.control[u] = v;
                            } else if r == nfe_vertex_streams_vertex_divisor(s) {
                                fe.divisor[u] = v;
                            } else if r == nfe_generic_attrib_config0(s) {
                                fe.cfg0[u] = v;
                            } else if r == nfe_generic_attrib_config1(s) {
                                fe.cfg1[u] = v;
                            }
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    fe.counts.push(w & 0xffff);
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    fe
}

fn draw4() -> Fe {
    let _ = take_kicks();
    // SAFETY: plain GL calls on the bound arrays.
    unsafe {
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4);
        glFinish();
    }
    no_error("glDrawArraysInstanced");
    front_end(&take_kicks())
}

/// Common to both cases: two streams, one per attribute, stride 32, each
/// element at the start of its own stream.
fn check_streams_and_elements(fe: &Fe) {
    assert_eq!(fe.control, [vertex_stream_stride(32); 2], "both streams carry the buffer's stride 32");
    assert_eq!(
        fe.base[1].wrapping_sub(fe.base[0]),
        16,
        "stream 1 starts 16 bytes behind stream 0: the attribute offset"
    );
    for k in 0..2usize {
        let c0 = fe.cfg0[k];
        assert_eq!(c0 & attrib_stream(0xf), attrib_stream(k as u32), "element {k} reads stream {k}");
        assert_eq!(c0 & 0xf, attrib_type(GC_DATA_TYPE_FLOAT), "element {k}: float");
        assert_eq!(c0 & attrib_num(0x3), attrib_num(4), "element {k}: four components");
        assert_eq!(c0 & attrib_start(0xff), attrib_start(0), "element {k} starts at 0 of its stream");
        assert_eq!(fe.cfg1[k] & 0xff, 16, "element {k}: sixteen bytes");
    }
}

/// Per-vertex attribute 0 and per-instance attribute 1 (divisor 2) in one
/// buffer: two streams, words 0 and 2, ONE DRAW_INSTANCED of the 4 instances.
#[test]
fn interleaved_per_vertex_and_per_instance_get_own_streams_and_divisors() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup(2);
        let fe = draw4();
        assert_eq!(fe.divisor, [0, 2], "divisor words of streams 0 and 1");
        assert_eq!(fe.counts, [4], "ONE DRAW_INSTANCED carrying the 4 instances");
        check_streams_and_elements(&fe);
        assert_ne!(fe.base[0], fe.base[1], "the two attributes are not one stream");
    });
}

/// Control: both attributes at divisor 0. Pinned: the back end still builds
/// one stream per attribute (it never merges by buffer and stride), both
/// words 0.
#[test]
fn control_both_per_vertex_are_still_two_streams() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup(0);
        let fe = draw4();
        assert_eq!(fe.divisor, [0, 0], "divisor words of streams 0 and 1");
        assert_eq!(fe.counts, [4], "ONE DRAW_INSTANCED carrying the 4 instances");
        check_streams_and_elements(&fe);
        assert_ne!(fe.base[0], fe.base[1], "pinned: not merged into one stream");
    });
}
