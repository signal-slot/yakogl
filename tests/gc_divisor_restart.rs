// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): GL_PRIMITIVE_RESTART_FIXED_INDEX with a mixed-divisor
//! glDrawElementsInstanced, after V1 (the hardware per-stream divisor,
//! docs/notes/instanceid-per-draw.md). tests/gc_stream_divisor_elements.rs
//! checks the same draw without primitive restart.
//!
//! Two arrays at glVertexAttribDivisor 0 and 2, a GL_TRIANGLE_STRIP element
//! list {0, 1, 2, R, 2, 1, 0} where R is the type's restart index (0xffff for
//! GL_UNSIGNED_SHORT, 0xff for GL_UNSIGNED_BYTE), 4 instances, restart on.
//!
//! How the restart reaches the hardware: the GL core (src/gl/draw.rs,
//! `restart_to_lists`) rewrites an index list that holds the restart index
//! into the independent list primitive the mode is made of, without the
//! restart index, and stages it as 16-bit indices. The back end then writes
//! GC_FE_PRIMITIVE_RESTART_INDEX 0 and no restart index ever reaches the
//! front end. So the draw is checked for what that design gives:
//!
//! - ONE indexed DRAW_INSTANCED of the 4 instances, GC_PRIM_TRIANGLES, 6
//!   indices (the two strip pieces as two triangles), not split per
//!   instance and not refused (`gc_refused_draws` unchanged);
//! - the divisor words at 0x14680 and 0x14684 loaded at that draw are 0 and 2;
//! - the index stream is GC_INDEX_TYPE_U16 and its memory holds
//!   {0, 1, 2, 2, 1, 0}: the restart index of either type is gone;
//! - GC_FE_PRIMITIVE_RESTART_INDEX is 0 (unused: nothing in the stream to
//!   match it).
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_divisor_restart
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawElementsInstanced;
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glEnable, glFinish, glGetError, glViewport};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribDivisor, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
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
        p.allocs.insert(gpu, Alloc { mem, size });
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

/// `n` 16-bit words at GPU address `gpu` of a live allocation (None: no
/// allocation holds them).
fn read_u16(gpu: u64, n: usize) -> Option<Vec<u16>> {
    with_platform(|p| {
        let (&base, a) = p.allocs.range(..=gpu).next_back()?;
        let off = (gpu - base) as usize;
        if off + 2 * n > a.size {
            return None;
        }
        // SAFETY: `off + 2n` bytes lie inside the allocation (checked above).
        let b = unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), 2 * n) };
        Some(b.as_chunks::<2>().0.iter().map(|q| u16::from_ne_bytes(*q)).collect())
    })
}

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

/// A linked program of `vs` and `fs`, attribute `p` at 0 and `q` at 1.
fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glBindAttribLocation(p, 1, c"q".as_ptr());
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

/* ---- the decoding ------------------------------------------------------------------ */

/// What one DRAW_INSTANCED sees: the state loaded before it in its stream
/// (u32::MAX: never written) and its own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Draw {
    /// The divisor words at 0x14680 and 0x14684.
    divisors: [u32; 2],
    index_base: u32,
    index_control: u32,
    restart_index: u32,
    indexed: bool,
    prim: u32,
    instances: u32,
    vertices: u32,
}

fn draws(kicks: &[Vec<u32>]) -> Vec<Draw> {
    let mut out = Vec::new();
    let div = [0, 1].map(nfe_vertex_streams_vertex_divisor);
    for words in kicks {
        let mut divisors = [u32::MAX; 2];
        let (mut index_base, mut index_control, mut restart_index) = (u32::MAX, u32::MAX, u32::MAX);
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let reg = base + 4 * k as u32;
                        let v = words[i + 1 + k];
                        if let Some(s) = div.iter().position(|&x| x == reg) {
                            divisors[s] = v;
                        }
                        match reg {
                            GC_FE_INDEX_STREAM_BASE_ADDR => index_base = v,
                            GC_FE_INDEX_STREAM_CONTROL => index_control = v,
                            GC_FE_PRIMITIVE_RESTART_INDEX => restart_index = v,
                            _ => {}
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    let c = words[i + 1];
                    out.push(Draw {
                        divisors,
                        index_base,
                        index_control,
                        restart_index,
                        indexed: w & 0x0010_0000 != 0,
                        prim: (w >> 16) & 0xf,
                        instances: (w & 0xffff) | ((c >> 24) << 16),
                        vertices: c & 0x00ff_ffff,
                    });
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    out
}

/* ---- the cases --------------------------------------------------------------------- */

const VS: &str = "attribute vec4 p;\nattribute vec4 q;\nvarying vec4 v;\n\
                  void main() { v = q; gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";

/// Positions (attribute 0) of a triangle, then four floats an instance pair
/// for attribute 1 (divisor 2: 4 instances read 2 elements).
static DATA: [f32; 20] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
];

/// Restart on, arrays at divisors 0 and 2, an element buffer holding `bytes`
/// (indices of type `ty`: the strip {0, 1, 2, R, 2, 1, 0}), then
/// glDrawElementsInstanced of the 7 indices and 4 instances.
fn run(ty: GLenum, bytes: &[u8]) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        // SAFETY: plain GL calls; DATA is a static, `bytes` outlives the calls.
        unsafe {
            glUseProgram(program(VS, FS));
            glViewport(0, 0, 64, 64);
            glEnable(GL_PRIMITIVE_RESTART_FIXED_INDEX);
            let mut bo = [0 as GLuint; 2];
            glGenBuffers(2, bo.as_mut_ptr());
            glBindBuffer(GL_ARRAY_BUFFER, bo[0]);
            glBufferData(
                GL_ARRAY_BUFFER,
                core::mem::size_of_val(&DATA) as GLsizeiptr,
                DATA.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, bo[1]);
            glBufferData(
                GL_ELEMENT_ARRAY_BUFFER,
                bytes.len() as GLsizeiptr,
                bytes.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            glEnableVertexAttribArray(0);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, (12 * 4) as *const c_void);
            glVertexAttribDivisor(0, 0);
            glVertexAttribDivisor(1, 2);
        }
        no_error("the setup");
        let _ = take_kicks();
        let refused = yakogl::vivante::backend::gc_refused_draws();
        // SAFETY: plain GL calls; the indices come from the bound buffer.
        unsafe {
            glDrawElementsInstanced(GL_TRIANGLE_STRIP, 7, ty, core::ptr::null(), 4);
            glFinish();
        }
        no_error("glDrawElementsInstanced");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws().wrapping_sub(refused), 0, "no draw refused");
        let d = draws(&take_kicks());
        assert_eq!(d.len(), 1, "ONE DRAW_INSTANCED of the 4 instances (not split per instance): {d:x?}");
        let d = d[0];
        assert_eq!(d.divisors, [0, 2], "words at 0x14680, 0x14684");
        assert!(d.indexed, "an indexed DRAW_INSTANCED: {d:x?}");
        assert_eq!(d.instances, 4, "the 4 instances");
        assert_eq!(d.prim, GC_PRIM_TRIANGLES, "the strip pieces as a triangle list");
        assert_eq!(d.vertices, 6, "two triangles: the restart index is not drawn");
        assert_eq!(d.index_control, GC_INDEX_TYPE_U16, "indices staged as 16 bits");
        assert_eq!(d.restart_index, 0, "GC_FE_PRIMITIVE_RESTART_INDEX unused");
        let ix = read_u16(u64::from(d.index_base), 6)
            .unwrap_or_else(|| panic!("index stream 0x{:x} is no allocation", d.index_base));
        assert_eq!(ix, [0, 1, 2, 2, 1, 0], "the two triangles, no restart index left");
    });
}

#[test]
fn unsigned_short_restart_mixed_divisors_is_one_indexed_instanced_draw() {
    let bytes: Vec<u8> = [0u16, 1, 2, 0xffff, 2, 1, 0].iter().flat_map(|i| i.to_ne_bytes()).collect();
    run(GL_UNSIGNED_SHORT, &bytes);
}

#[test]
fn unsigned_byte_restart_mixed_divisors_is_one_indexed_instanced_draw() {
    run(GL_UNSIGNED_BYTE, &[0, 1, 2, 0xff, 2, 1, 0]);
}
