// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the vertex-count field of DRAW_INSTANCED.
//! Every GC draw, plain or instanced, goes through `Cmd::draw_instanced`
//! (src/vivante/cmd.rs:122); `draw_instanced_count` (src/vivante/regs.rs:37)
//! keeps only the low 24 bits of the vertex count (maximum 0xffffff).
//!
//! * glDrawArrays and glDrawArraysInstanced (2 instances) of GL_POINTS with
//!   1, 0xffff, 0x10000 and 0xffffff vertices are each ONE DRAW_INSTANCED
//!   with exactly those words;
//! * 0x1000000 vertices do not fit. `draw_parts` (src/vivante/backend.rs)
//!   splits glDrawArrays of points into 0xffffff + 1 vertices with `first`
//!   advancing; the instanced draw cannot be split by vertex and is refused
//!   (counted in `gc_refused_draws`, a GL error, no command). Never a
//!   command carrying the count modulo 2^24 (an empty draw).
//! * glDrawElements of 0x1000000 indices is refused the same way: the third
//!   word of an indexed DRAW_INSTANCED is the index bias, not the start, so
//!   a part could not start further into the index list. 0xffffff indices
//!   are one command.
//!
//! No vertex buffer is built: attribute 0 stays DISABLED (the current value
//! feeds the shader), so 16M vertices need no memory. The stand-in OSAL
//! executes nothing, so large counts cost no time.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_vertex_count_wide
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::{glDrawArrays, glDrawArraysInstanced, glDrawElements};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glFinish, glGetError, glViewport};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};
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

const VS: &str =
    "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = vec4(1.0); gl_Position = p; gl_PointSize = 1.0; }\n";
const FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";

/// One DRAW_INSTANCED as the FE decodes it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Draw {
    instances: u32,
    vertices: u32,
    first: u32,
}

fn draws(kicks: &[Vec<u32>]) -> Vec<Draw> {
    let mut out = Vec::new();
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => i += 1 + ((w >> 16) & 0x3ff) as usize,
                GC_OP_DRAW_INSTANCED => {
                    out.push(Draw {
                        instances: (w & 0xffff) | ((words[i + 1] >> 24) << 16),
                        vertices: words[i + 1] & 0xff_ffff,
                        first: words[i + 2],
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

/// What one draw call did: the DRAW_INSTANCED commands emitted, its GL
/// error and how many draws the back end refused (`gc_refused_draws`).
type Outcome = (Vec<Draw>, GLenum, u32);

/// After the setup, call `issue(0)` .. `issue(calls - 1)` (one draw call
/// each) in one context; the outcome of each.
fn run(calls: usize, issue: impl Fn(usize)) -> Vec<Outcome> {
    let mut out = Vec::new();
    with_backend(|| {
        // SAFETY: plain GL calls; attribute 0 stays disabled.
        unsafe {
            glUseProgram(program(VS, FS));
            glViewport(0, 0, 64, 64);
        }
        no_error("the setup");
        for k in 0..calls {
            let _ = take_kicks();
            let base = gc_refused_draws();
            issue(k);
            // SAFETY: plain GL calls.
            let e = unsafe {
                glFinish();
                glGetError()
            };
            out.push((draws(&take_kicks()), e, gc_refused_draws().wrapping_sub(base)));
        }
    });
    out
}

/// Draw `n` points (`inst` == 0: glDrawArrays, else glDrawArraysInstanced).
fn draw(n: GLsizei, inst: GLsizei) -> Outcome {
    // SAFETY: plain GL calls.
    let mut out = run(1, |_| unsafe {
        if inst == 0 {
            glDrawArrays(GL_POINTS, 0, n);
        } else {
            glDrawArraysInstanced(GL_POINTS, 0, n, inst);
        }
    });
    out.remove(0)
}

/// glDrawElements of GL_UNSIGNED_SHORT points, every index 0, from a client
/// array, once for each count in `counts` in one context (the GL core's
/// index staging outlives a context, so it is not handed a fresh stand-in).
fn draw_elements(counts: &[usize]) -> Vec<Outcome> {
    let indices = vec![0u16; counts.iter().copied().max().unwrap_or(0)];
    // SAFETY: `indices` holds at least `counts[k]` u16s and outlives the calls.
    run(counts.len(), |k| unsafe {
        glDrawElements(GL_POINTS, counts[k] as GLsizei, GL_UNSIGNED_SHORT, indices.as_ptr() as *const c_void)
    })
}

fn check(n: GLsizei, inst: GLsizei) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (d, e, r) = draw(n, inst);
    assert_eq!(r, 0, "{n:#x} vertices x {inst}: refused");
    assert_eq!(e, GL_NO_ERROR, "{n:#x} vertices x {inst}: GL error 0x{e:x}");
    assert_eq!(d.len(), 1, "{n:#x} vertices x {inst}: ONE DRAW_INSTANCED, got {d:?}");
    assert_eq!(d[0].vertices, n as u32, "{n:#x} vertices x {inst}: {d:?}");
    assert_eq!(d[0].first, 0, "{d:?}");
    assert_eq!(d[0].instances, inst.max(1) as u32, "{d:?}");
}

#[test]
fn one_vertex() {
    check(1, 0);
    check(1, 2);
}

#[test]
fn count_0xffff_fills_the_low_16_bits() {
    check(0xffff, 0);
    check(0xffff, 2);
}

#[test]
fn count_0x10000_crosses_bit_16() {
    check(0x10000, 0);
    check(0x10000, 2);
}

#[test]
fn count_0xffffff_is_the_field_maximum() {
    check(0xff_ffff, 0);
    check(0xff_ffff, 2);
}

/// 0x1000000 vertices do not fit the 24-bit field. glDrawArrays of points
/// is split on the field maximum with `first` advancing; the instanced draw
/// cannot be split by vertex and is refused (a GL error, counted, no
/// command). Never a command carrying the truncated count. Revert check:
/// without `draw_parts` both emitted one command of 0 vertices and no error.
#[test]
fn count_above_the_field_maximum_is_split_or_refused() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (d, e, r) = draw(0x100_0000, 0);
    assert_eq!((e, r), (GL_NO_ERROR, 0), "glDrawArrays: GL error 0x{e:x}, {r} refused");
    assert_eq!(
        d,
        [Draw { instances: 1, vertices: 0xff_ffff, first: 0 }, Draw { instances: 1, vertices: 1, first: 0xff_ffff }],
        "glDrawArrays: split at the field maximum"
    );

    let (d, e, r) = draw(0x100_0000, 2);
    assert!(d.is_empty(), "instanced: no command, got {d:?}");
    assert_ne!(e, GL_NO_ERROR, "instanced: no draw and no error: silently dropped");
    assert_eq!(r, 1, "instanced: one counted refusal");
}

/// An indexed draw past the vertex field is refused end to end: a part
/// would need its start in the index stream's base, which is written once
/// per draw (the third word is the index bias). 0xffffff indices (the same
/// staging, two bytes fewer) are one command, so the refusal is the
/// count's, not the memory's. Revert check: with `d.first + v` in each
/// part, the parts re-read the indices from 0 with a wrong bias.
#[test]
fn indexed_count_above_the_field_maximum_is_refused() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = draw_elements(&[0xff_ffff, 0x100_0000]).into_iter();
    let (d, e, r) = out.next().unwrap();
    assert_eq!((e, r), (GL_NO_ERROR, 0), "0xffffff indices: GL error 0x{e:x}, {r} refused");
    assert_eq!(d.len(), 1, "0xffffff indices: ONE DRAW_INSTANCED, got {d:?}");
    assert_eq!((d[0].vertices, d[0].instances), (0xff_ffff, 1), "{d:?}");

    let (d, e, r) = out.next().unwrap();
    assert!(d.is_empty(), "0x1000000 indices: no command, got {d:?}");
    assert_ne!(e, GL_NO_ERROR, "0x1000000 indices: no draw and no error: silently dropped");
    assert_eq!(r, 1, "0x1000000 indices: one counted refusal");
}
