// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): per-instance integer arrays keep their integer format
//! and their divisor in the command stream (V1, docs/notes/instanceid-per-draw.md,
//! with the typed attributes of Appendix A T1).
//!
//! A program with a `vec4` input at location 0 (array, divisor 0), an `ivec4`
//! at 1 and a `uvec4` at 2 (both glVertexAttribIPointer arrays, divisors 2
//! and 1), drawn with glDrawArraysInstanced of 3 instances:
//!
//! * the divisor words of streams 0, 1 and 2 (0x14680, 0x14684, 0x14688)
//!   are 0, 2 and 1;
//! * the vertex element words (`nfe_generic_attrib_config0(i)`) of the two
//!   integer arrays are the 32-bit four-component fetch the back end uses
//!   for every 4-byte input (GC_DATA_TYPE_FLOAT, no normalisation, no
//!   sign-extend): the integer type is the register's bit pattern and the
//!   fetch converts nothing to float;
//! * the three instances are ONE DRAW_INSTANCED, not split, and nothing is
//!   refused.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_divisor_int
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

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
    glEnableVertexAttribArray, glVertexAttribDivisor, glVertexAttribIPointer, glVertexAttribPointer,
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

/* ---- the case ---------------------------------------------------------------------- */

const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 p;\nlayout(location = 1) in highp ivec4 q;\n\
                  layout(location = 2) in highp uvec4 r;\nflat out highp ivec4 v;\n\
                  void main() { v = q + ivec4(r); gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nflat in highp ivec4 v;\nout vec4 o;\n\
                  void main() { o = vec4(v); }\n";

/// Positions of a triangle, then 3 ivec4 and 3 uvec4 (a value per instance).
static POS: [f32; 12] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0];
static INTS: [i32; 12] = [-1, 2, -3, 4, 5, -6, 7, -8, i32::MIN, i32::MAX, 0, 1];
static UINTS: [u32; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 0xffff_ffff, 0x8000_0000, 0, 9];

/// Per DRAW_INSTANCED: the divisor words of streams 0..3, the config words 0
/// of elements 0..3, and the instance count.
type Seen = (Vec<[u32; 3]>, Vec<[u32; 3]>, Vec<u32>);

fn last_words(kicks: &[Vec<u32>]) -> Seen {
    let div = [0, 1, 2].map(nfe_vertex_streams_vertex_divisor);
    let cfg = [0, 1, 2].map(nfe_generic_attrib_config0);
    let (mut divs, mut cfgs, mut counts) = (Vec::new(), Vec::new(), Vec::new());
    for words in kicks {
        let (mut d, mut c) = ([u32::MAX; 3], [u32::MAX; 3]);
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let r = base + 4 * k as u32;
                        if let Some(s) = div.iter().position(|&x| x == r) {
                            d[s] = words[i + 1 + k];
                        }
                        if let Some(s) = cfg.iter().position(|&x| x == r) {
                            c[s] = words[i + 1 + k];
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    divs.push(d);
                    cfgs.push(c);
                    counts.push(w & 0xffff);
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    (divs, cfgs, counts)
}

#[test]
fn per_instance_integer_arrays_keep_format_and_divisor_in_one_draw() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        // SAFETY: plain GL calls; the arrays are statics.
        unsafe {
            glUseProgram(program(VS, FS));
            glViewport(0, 0, 64, 64);
            let mut vbo: [GLuint; 3] = [0; 3];
            glGenBuffers(3, vbo.as_mut_ptr());
            let data: [(*const c_void, usize); 3] = [
                (POS.as_ptr() as *const c_void, core::mem::size_of_val(&POS)),
                (INTS.as_ptr() as *const c_void, core::mem::size_of_val(&INTS)),
                (UINTS.as_ptr() as *const c_void, core::mem::size_of_val(&UINTS)),
            ];
            for (b, (p, n)) in vbo.iter().zip(data) {
                glBindBuffer(GL_ARRAY_BUFFER, *b);
                glBufferData(GL_ARRAY_BUFFER, n as GLsizeiptr, p, GL_STATIC_DRAW);
            }
            for i in 0..3 {
                glEnableVertexAttribArray(i);
            }
            glBindBuffer(GL_ARRAY_BUFFER, vbo[0]);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glBindBuffer(GL_ARRAY_BUFFER, vbo[1]);
            glVertexAttribIPointer(1, 4, GL_INT, 0, core::ptr::null());
            glBindBuffer(GL_ARRAY_BUFFER, vbo[2]);
            glVertexAttribIPointer(2, 4, GL_UNSIGNED_INT, 0, core::ptr::null());
            glVertexAttribDivisor(1, 2);
            glVertexAttribDivisor(2, 1);
        }
        no_error("the setup");
        let _ = take_kicks();
        let refused = yakogl::vivante::backend::gc_refused_draws();
        // SAFETY: plain GL calls on the bound arrays.
        unsafe {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 3);
            glFinish();
        }
        no_error("glDrawArraysInstanced");
        assert_eq!(yakogl::vivante::backend::gc_refused_draws().wrapping_sub(refused), 0, "nothing refused");
        let (divs, cfgs, counts) = last_words(&take_kicks());
        assert_eq!(counts, [3], "ONE hardware draw of the 3 instances, not split");
        assert_eq!(divs, [[0, 2, 1]], "divisor words of streams 0, 1, 2");
        let cfg = cfgs[0];
        for (i, what) in [(0usize, "float array"), (1, "ivec4 array"), (2, "uvec4 array")] {
            assert_eq!(cfg[i] & 0xf, GC_DATA_TYPE_FLOAT, "{what}: 32-bit fetch, no conversion");
            assert_eq!(cfg[i] & (0x3 << 12), attrib_num(4), "{what}: four components");
            assert_eq!(cfg[i] & (0x3 << 14), attrib_normalize(GC_NORMALIZE_OFF), "{what}: not normalised");
        }
    });
}
