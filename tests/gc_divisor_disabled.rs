// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a DISABLED generic attribute with a divisor set
//! (glVertexAttribDivisor(1, 3)) next to V1's stream divisor words
//! (`emit_streams`, src/vivante/backend.rs; docs/notes/instanceid-per-draw.md).
//!
//! ES 3.0: the divisor of a disabled attribute has no effect, the shader
//! reads the current value (glVertexAttrib4f). The GL core stages a disabled
//! input as a zero-stride stream holding that value (tests/gc_current_attrib_stage.rs).
//! A glDrawArraysInstanced of 4 instances with attribute 0 per vertex checks:
//!
//! * ONE DRAW_INSTANCED of 4 instances, `gc_refused_draws` unchanged;
//! * stream 1 has stride 0 and holds the current value; its divisor word is
//!   whatever the back end emits. With stride 0 every instance reads the
//!   same 16 bytes at any divisor, so the word cannot change the picture; the
//!   test pins what is emitted (0: the staged stream is per vertex, the
//!   array's divisor is not copied into it);
//! * enabling attribute 1 on a real buffer makes the divisor word 3.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_divisor_disabled
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
use yakogl::gl::vertex::{
    glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttrib4f, glVertexAttribDivisor,
    glVertexAttribPointer,
};
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

/// Positions (attribute 0) of a triangle, then four floats an instance.
static DATA: [f32; 24] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0,
];

/// Every register written in the streams since the last call, last write
/// winning, and the instance counts of the DRAW_INSTANCED commands.
fn final_state_and_draws(kicks: &[Vec<u32>]) -> (BTreeMap<u32, u32>, Vec<u32>) {
    let (mut regs, mut counts) = (BTreeMap::new(), Vec::new());
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        regs.insert(base + 4 * k as u32, words[i + 1 + k]);
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    counts.push(w & 0xffff);
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    (regs, counts)
}

/// The 16 bytes at GPU address `gpu`, as the four dwords the stream fetches.
fn fetch16(gpu: u32) -> [u32; 4] {
    with_platform(|p| {
        let (&base, a) = p.allocs.range(..=u64::from(gpu)).next_back().expect("a stream base inside no allocation");
        let off = (u64::from(gpu) - base) as usize;
        assert!(off + 16 <= a.mem.len() * 8, "stream base 0x{gpu:x} runs off its allocation");
        // SAFETY: bounds checked above; the allocation is plain host memory.
        let b = unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), 16) };
        core::array::from_fn(|k| u32::from_ne_bytes(b[4 * k..4 * k + 4].try_into().unwrap()))
    })
}

/// A disabled attribute 1 with divisor 3 and the current value (0.25, 0.5,
/// 0.75, 1), then the same attribute enabled on a buffer.
#[test]
fn a_disabled_attribute_ignores_its_divisor_and_an_enabled_one_gets_word_three() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
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
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            /* attribute 1: a pointer into the buffer (must not be read), a
             * divisor, but left disabled */
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, (12 * 4) as *const c_void);
            glVertexAttribDivisor(1, 3);
            glDisableVertexAttribArray(1);
            glVertexAttrib4f(1, 0.25, 0.5, 0.75, 1.0);
        }
        no_error("the setup");

        let _ = take_kicks();
        let refused = yakogl::vivante::backend::gc_refused_draws();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4);
            glFinish();
        }
        no_error("glDrawArraysInstanced, attribute 1 disabled");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws().wrapping_sub(refused), 0, "nothing refused");
        let (regs, counts) = final_state_and_draws(&take_kicks());
        assert_eq!(counts, [4], "ONE instanced draw of the 4 instances");
        assert_eq!(regs[&nfe_vertex_streams_vertex_divisor(0)], 0, "stream 0 per vertex");
        let base = *regs.get(&nfe_vertex_streams_base_addr(1)).expect("stream 1 has no base");
        assert_eq!(
            fetch16(base),
            [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits),
            "the staged stream is the current value, not buffer data"
        );
        assert_eq!(regs[&nfe_vertex_streams_control(1)], vertex_stream_stride(0), "zero stride");
        /* The GL core stages the disabled input as a plain zero-stride,
         * divisor-0 stream; the divisor is not copied. Any word would read
         * the same 16 bytes at stride 0, so 0 is just what is emitted. */
        assert_eq!(regs[&nfe_vertex_streams_vertex_divisor(1)], 0, "divisor word of the disabled input");

        // Enable it: a real per-instance array, divisor 3.
        // SAFETY: plain GL call.
        unsafe { glEnableVertexAttribArray(1) };
        let _ = take_kicks();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4);
            glFinish();
        }
        no_error("glDrawArraysInstanced, attribute 1 enabled");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws().wrapping_sub(refused), 0, "nothing refused");
        let (regs, counts) = final_state_and_draws(&take_kicks());
        assert_eq!(counts, [4], "ONE instanced draw of the 4 instances");
        assert_eq!(regs[&nfe_vertex_streams_vertex_divisor(0)], 0, "stream 0 per vertex");
        assert_eq!(regs[&nfe_vertex_streams_vertex_divisor(1)], 3, "the enabled array's divisor");
        assert_ne!(regs[&nfe_vertex_streams_control(1)], vertex_stream_stride(0), "a real stride now");
    });
}
