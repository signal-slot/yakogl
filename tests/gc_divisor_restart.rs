// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): primitive restart together with mixed stream divisors in
//! one glDrawElementsInstanced (V1, docs/notes/instanceid-per-draw.md).
//!
//! GL_PRIMITIVE_RESTART_FIXED_INDEX on, arrays at divisors 0 and 2, an element
//! buffer holding the restart index (0xffff for GL_UNSIGNED_SHORT, 0xff for
//! GL_UNSIGNED_BYTE): the stream must have ONE indexed DRAW_INSTANCED of all
//! the instances, the stream divisor words 0 and 2, and no refused draw.
//!
//! How it runs: as tests/gc_stream_divisor.rs (real GL core and Vivante back
//! end over a stand-in OSAL that keeps every submitted stream).
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

const VS2: &str = "attribute vec4 p;\nattribute vec4 q;\nvarying vec4 v;\n\
                   void main() { v = q; gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";

/// Positions (attribute 0) of three vertices, then four floats an instance
/// for attribute 1.
static DATA: [f32; 24] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0,
];

/// What the streams hold: per DRAW_INSTANCED the divisor words of streams 0
/// and 1 loaded before it (u32::MAX: never written), its indexed bit and its
/// instance count; and the last words written to GC_FE_PRIMITIVE_RESTART_INDEX
/// (None: never written).
struct Seen {
    divisors: Vec<[u32; 2]>,
    cmds: Vec<(bool, u32)>,
    restart_index: Option<u32>,
}

fn decode(kicks: &[Vec<u32>]) -> Seen {
    let mut s = Seen { divisors: vec![], cmds: vec![], restart_index: None };
    let regs = [0, 1].map(nfe_vertex_streams_vertex_divisor);
    for words in kicks {
        let mut cur = [u32::MAX; 2];
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let (reg, val) = (base + 4 * k as u32, words[i + 1 + k]);
                        if let Some(n) = regs.iter().position(|&x| x == reg) {
                            cur[n] = val;
                        } else if reg == GC_FE_PRIMITIVE_RESTART_INDEX {
                            s.restart_index = Some(val);
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    s.divisors.push(cur);
                    s.cmds.push((w & 0x0010_0000 != 0, w & 0xffff));
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    s
}

/// Attributes 0 (divisor 0) and 1 (divisor 2), primitive restart on, an
/// element buffer holding `bytes` (indices of type `ty`), then a
/// GL_TRIANGLE_STRIP glDrawElementsInstanced of `n` indices and 4 instances.
fn run(ty: GLenum, bytes: &[u8], n: GLsizei) -> Seen {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        // SAFETY: plain GL calls; DATA is a static, `bytes` outlives the calls.
        unsafe {
            glUseProgram(program(VS2, FS));
            glViewport(0, 0, 64, 64);
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
            glEnable(GL_PRIMITIVE_RESTART_FIXED_INDEX);
        }
        no_error("the setup");
        let _ = take_kicks();
        let refused = yakogl::vivante::backend::gc_refused_draws();
        // SAFETY: plain GL calls; the indices come from the bound buffer.
        unsafe {
            glDrawElementsInstanced(GL_TRIANGLE_STRIP, n, ty, core::ptr::null(), 4);
            glFinish();
        }
        no_error("glDrawElementsInstanced");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws().wrapping_sub(refused), 0, "no draw refused");
        decode(&take_kicks())
    })
}

/// The claims common to both types: one indexed DRAW_INSTANCED of the 4
/// instances, the divisor words 0 and 2.
fn check_one_draw(s: &Seen) {
    assert_eq!(s.divisors, [[0, 2]], "stream divisor words at 0x14680 and 0x14684");
    assert_eq!(s.cmds, [(true, 4)], "ONE indexed DRAW_INSTANCED of the 4 instances (not split per instance)");
}

#[test]
fn unsigned_short_restart_mixed_divisors_is_one_indexed_instanced_draw() {
    let ix: [u16; 7] = [0, 1, 2, 0xffff, 0, 1, 2];
    let bytes: Vec<u8> = ix.iter().flat_map(|i| i.to_ne_bytes()).collect();
    check_one_draw(&run(GL_UNSIGNED_SHORT, &bytes, 7));
}

#[test]
fn unsigned_byte_restart_mixed_divisors_is_one_indexed_instanced_draw() {
    check_one_draw(&run(GL_UNSIGNED_BYTE, &[0, 1, 2, 0xff, 0, 1, 2], 7));
}

/// The item's contract: the draw has restart enabled with index 0xffff. The GL
/// core lowers the restart on the CPU (src/gl/draw.rs, "ES 3.0 2.8.1") and the
/// back end writes GC_FE_PRIMITIVE_RESTART_INDEX as 0 (src/vivante/backend.rs
/// emit_draw), so the stream holds no restart index.
#[test]
#[ignore = "no hardware primitive restart on the Vivante back end: the GL core lowers it to list indices and the index word is 0"]
fn unsigned_short_restart_index_is_0xffff_in_the_stream() {
    let ix: [u16; 7] = [0, 1, 2, 0xffff, 0, 1, 2];
    let bytes: Vec<u8> = ix.iter().flat_map(|i| i.to_ne_bytes()).collect();
    assert_eq!(run(GL_UNSIGNED_SHORT, &bytes, 7).restart_index, Some(0xffff));
}

#[test]
#[ignore = "no hardware primitive restart on the Vivante back end: the GL core lowers it to list indices and the index word is 0"]
fn unsigned_byte_restart_index_is_0xff_in_the_stream() {
    assert_eq!(run(GL_UNSIGNED_BYTE, &[0, 1, 2, 0xff, 0, 1, 2], 7).restart_index, Some(0xff));
}
