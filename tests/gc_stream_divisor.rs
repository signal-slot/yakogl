// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the per-stream vertex divisor words of the command
//! stream (`nfe_vertex_streams_vertex_divisor(i)`, `emit_streams` in
//! src/vivante/backend.rs), pinned for the draws the hardware divisor (V1 of
//! docs/notes/instanceid-per-draw.md) must leave as they are.
//!
//! * A glDrawArrays with two per-vertex attributes: the words of streams 0
//!   and 1 are 0.
//! * A glDrawArraysInstanced with one per-vertex attribute and one at
//!   glVertexAttribDivisor(1, 1): the per-vertex stream's word is 0, the
//!   per-instance one's is 1, and the instances are ONE DRAW_INSTANCED that
//!   carries all of them, not a draw per instance.
//!
//! And the draws V1 changes (`max_instance_divisor` 0xffff, the divisor
//! itself in the stream's word):
//!
//! * A mixed-divisor glDrawArraysInstanced and glDrawElementsInstanced
//!   (attributes at divisors 0, 1 and 3, 4 instances): ONE DRAW_INSTANCED of
//!   the 4 instances, the three streams' words 0, 1 and 3.
//! * An attribute at divisor 0x10000, above what the table admits: the GL
//!   core still draws instance by instance (one DRAW_INSTANCED of one
//!   instance each, the element a constant, no stream word above 0).
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_state_dedup.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_stream_divisor
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::{glDrawArrays, glDrawArraysInstanced, glDrawElementsInstanced};
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

/// A linked program of `vs` and `fs`, attribute `p` at 0, `q` at 1 and `r`
/// (when the vertex shader has one) at 2.
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

/// Positions (attribute 0) of a triangle, then four floats a vertex or an
/// instance for attribute 1.
static DATA: [f32; 24] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0,
];

/// The program and a buffer of [`DATA`] with attribute 0 at its start and
/// attribute 1 behind the positions, bound and enabled.
fn setup() {
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
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, (12 * 4) as *const c_void);
    }
    no_error("the setup");
}

/// The divisor words of streams 0..N as the stream has loaded them at each
/// DRAW_INSTANCED (u32::MAX: never written), and the instance counts of those
/// commands.
fn divisor_words_and_draws<const N: usize>(kicks: &[Vec<u32>]) -> (Vec<[u32; N]>, Vec<u32>) {
    let (mut pairs, mut counts) = (Vec::new(), Vec::new());
    let reg0 = nfe_vertex_streams_vertex_divisor(0);
    for words in kicks {
        let mut cur = [u32::MAX; N];
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let r = base + 4 * k as u32;
                        if r >= reg0 && (r - reg0) % 4 == 0 && ((r - reg0) / 4) < N as u32 {
                            cur[((r - reg0) / 4) as usize] = words[i + 1 + k];
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    pairs.push(cur);
                    counts.push(w & 0xffff);
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    (pairs, counts)
}

#[test]
fn per_vertex_streams_have_divisor_zero() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup();
        let _ = take_kicks();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        no_error("glDrawArrays");
        let (pairs, counts) = divisor_words_and_draws(&take_kicks());
        assert_eq!(pairs, [[0, 0]], "divisor words of streams 0 and 1, as loaded at the draw");
        assert_eq!(counts, [1], "one draw of one instance");
    });
}

#[test]
fn divisor_one_stream_has_word_one_in_one_instanced_draw() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup();
        // SAFETY: plain GL call.
        unsafe { glVertexAttribDivisor(1, 1) };
        let _ = take_kicks();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3);
            glFinish();
        }
        no_error("glDrawArraysInstanced");
        let (pairs, counts) = divisor_words_and_draws(&take_kicks());
        assert_eq!(pairs, [[0, 1]], "stream 0 per vertex, stream 1 per instance");
        assert_eq!(counts, [3], "ONE instanced draw of the 3 instances, not one draw per instance");
    });
}

/* ---- V1: the divisor itself, and the split that is left ---------------------------- */

const VS3: &str = "attribute vec4 p;\nattribute vec4 q;\nattribute vec4 r;\nvarying vec4 v;\n\
                   void main() { v = q + r; gl_Position = p; }\n";

/// Positions (attribute 0) of a triangle, then four vec4 for attribute 1 and
/// four for attribute 2: enough for 4 instances at any divisor.
static DATA3: [f32; 44] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, //
    0.5, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.5, 0.5, 0.5, 1.0,
];

/// The three-attribute program and a buffer of [`DATA3`], attribute 0 at its
/// start, 1 behind the positions and 2 behind those, bound and enabled, with
/// `divisors` set on them.
fn setup3(divisors: [GLuint; 3]) {
    // SAFETY: plain GL calls; DATA3 is a static.
    unsafe {
        glUseProgram(program(VS3, FS));
        glViewport(0, 0, 64, 64);
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&DATA3) as GLsizeiptr,
            DATA3.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        for (i, (off, div)) in [0usize, 12, 28].into_iter().zip(divisors).enumerate() {
            glEnableVertexAttribArray(i as GLuint);
            glVertexAttribPointer(i as GLuint, 4, GL_FLOAT, 0, 0, (off * 4) as *const c_void);
            glVertexAttribDivisor(i as GLuint, div);
        }
    }
    no_error("the setup");
}

/// An element buffer of the triangle's three indices, bound.
fn bind_indices() {
    static IDX: [u16; 3] = [0, 1, 2];
    // SAFETY: plain GL calls; IDX is a static.
    unsafe {
        let mut ibo: GLuint = 0;
        glGenBuffers(1, &mut ibo);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
        glBufferData(
            GL_ELEMENT_ARRAY_BUFFER,
            core::mem::size_of_val(&IDX) as GLsizeiptr,
            IDX.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
    }
    no_error("the index buffer");
}

/// V1 (docs/notes/instanceid-per-draw.md): a mixed-divisor instanced draw,
/// attributes at divisors 0, 1 and 3 and 4 instances, is ONE DRAW_INSTANCED
/// of the 4 instances (the table's `max_instance_divisor` 0xffff keeps the GL
/// core from splitting it), and each stream's divisor word is its array's
/// divisor: 0, 1 and 3, not a per-instance flag. Through glDrawArraysInstanced.
#[test]
fn mixed_divisors_are_each_streams_own_word_in_one_draw_arrays_instanced() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup3([0, 1, 3]);
        let _ = take_kicks();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 4);
            glFinish();
        }
        no_error("glDrawArraysInstanced");
        let (words, counts) = divisor_words_and_draws(&take_kicks());
        assert_eq!(words, [[0, 1, 3]], "the divisor words of streams 0, 1 and 2, as loaded at the draw");
        assert_eq!(counts, [4], "ONE instanced draw of the 4 instances, not one draw per instance");
    });
}

/// The same draw through glDrawElementsInstanced.
#[test]
fn mixed_divisors_are_each_streams_own_word_in_one_draw_elements_instanced() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup3([0, 1, 3]);
        bind_indices();
        let _ = take_kicks();
        // SAFETY: plain GL calls on the bound arrays and index buffer.
        unsafe {
            glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null(), 4);
            glFinish();
        }
        no_error("glDrawElementsInstanced");
        let (words, counts) = divisor_words_and_draws(&take_kicks());
        assert_eq!(words, [[0, 1, 3]], "the divisor words of streams 0, 1 and 2, as loaded at the draw");
        assert_eq!(counts, [4], "ONE instanced draw of the 4 instances, not one draw per instance");
    });
}

/// The split that V1 leaves: an attribute at a divisor above what the table
/// admits (0x10000 > `max_instance_divisor` 0xffff) is still drawn instance by
/// instance by the GL core, each instance ONE DRAW_INSTANCED of one instance
/// with that array's element as a constant, so no stream word is above 0.
/// (The back end refuses such an instance when the program reads
/// gl_InstanceID, src/vivante/backend_tests.rs; this program does not.)
#[test]
fn a_divisor_above_the_tables_is_still_drawn_instance_by_instance() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        setup3([0, 1, 0x10000]);
        let _ = take_kicks();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3);
            glFinish();
        }
        no_error("glDrawArraysInstanced");
        let (words, counts) = divisor_words_and_draws::<3>(&take_kicks());
        assert_eq!(counts, [1, 1, 1], "one draw of one instance per instance");
        assert!(
            words.iter().all(|w| w.iter().all(|&d| d == 0 || d == u32::MAX)),
            "no stream fetched per instance: {words:?}"
        );
    });
}
