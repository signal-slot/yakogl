// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the tiled-target work of 9cdeff6d (vivante:
//! tiled 32-bit render targets and early depth behind `gc_tiled_rt`) and
//! be0c1527 (dedup's PE room holds the tiled-target TS words). The old
//! i.MX8MP whole dEQP-GLES2 tree (0fbe61a3) predates both, and its stage-1
//! promise is 0 Fail. The i.MX8MP dEQP-GLES2 images build with the GC7000
//! back end, the online compiler and `report-es2`, and leave the
//! `gc_tiled_rt` knob off (a test knob, default off: src/vivante/tiled_rt.rs).
//!
//! For a scissored colour clear, a whole clear, and a draw, into the default
//! RGBA8888 target and into an RGB565 one, in a `report-es2` context:
//!
//! * no tile-status register is written (TS_COLOR_/TS_DEPTH_ STATUS_BASE,
//!   SURFACE_BASE, CLEAR_VALUE(_EXT), AUTO_DISABLE_COUNT, TS_SAMPLER_*),
//!   and PE_DEPTH_CONFIG never asks for early depth;
//! * the whole command stream of each case hashes to the value it had at
//!   9cdeff6d^ (the parent of the tiled work; the stand-in OSAL's bump
//!   allocator makes the addresses repeatable), word for word.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,no-shader-table,report-es2 --test gc_es2ctx_tiled_ts
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GLES_SURF_RGB565, GLES_SURF_RGBA8888};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glClear, glEnable, glFinish, glGetError, glGetIntegerv, glScissor, glViewport};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_debug_rs_clear, gc_init, gc_shutdown, yakogl_backend_vivante};
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
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are
/// process-global.
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
/// fresh 64x64 context whose default target is `fmt` (the core's own
/// RGBA8888 one, or a 565 surface the test allocates).
fn with_backend<R>(fmt: u8, case: impl FnOnce() -> R) -> R {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: SERIAL is held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        gc_debug_rs_clear(true);
        let c = if fmt == GLES_SURF_RGBA8888 {
            yakogl::gl::context::gl_context_create(64, 64)
        } else {
            let mut surf = OsalGpuMem::ZERO;
            assert_eq!(p_gpu_alloc(0x4000_0000, 64 * 64 * 2, 0, &mut surf), 0);
            yakogl::gl::context::gl_context_create_for_surface(
                &mut surf,
                64,
                64,
                fmt,
                1,
                yakogl::gl::context::GROUP_NONE,
            )
        };
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

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0, 0.5, 0.25, 0.5); }\n";

/// One triangle's positions.
static TRI: [f32; 12] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0];

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

/// A program and a buffer of [`TRI`] at attribute 0, bound and enabled.
fn setup() {
    // SAFETY: plain GL calls; TRI is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glViewport(0, 0, 64, 64);
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&TRI) as GLsizeiptr,
            TRI.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
    }
    no_error("the setup");
}

/// What the case does.
#[derive(Clone, Copy, Debug)]
enum Case {
    /// A clear under a scissor box smaller than the target: the clear quad.
    ScissoredClear,
    /// A whole clear: the resolve engine's fill.
    WholeClear,
    /// One triangle, no clear.
    Draw,
    /// A whole clear, then a triangle (one frame of a plain dEQP case).
    ClearThenDraw,
}

/// The streams `case` submits into a `fmt` default target.
fn streams(fmt: u8, case: Case) -> Vec<Vec<u32>> {
    with_backend(fmt, || {
        // SAFETY: plain GL calls.
        unsafe {
            let mut major = 0;
            glGetIntegerv(GL_MAJOR_VERSION, &mut major);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
            assert_eq!(major, 0);
        }
        setup();
        with_platform(|p| p.kicks.clear());
        // SAFETY: plain GL calls; setup() bound the program and the array.
        unsafe {
            match case {
                Case::ScissoredClear => {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(0, 0, 16, 16);
                    glClear(GL_COLOR_BUFFER_BIT);
                }
                Case::WholeClear => glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT),
                Case::Draw => glDrawArrays(GL_TRIANGLES, 0, 3),
                Case::ClearThenDraw => {
                    glClear(GL_COLOR_BUFFER_BIT);
                    glDrawArrays(GL_TRIANGLES, 0, 3);
                }
            }
            glFinish();
        }
        no_error("the case");
        with_platform(|p| core::mem::take(&mut p.kicks))
    })
}

/// Every `(register byte address, value)` a stream's LOAD_STATEs write.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    out.push((base + 4 * k as u32, words[i + 1 + k]));
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => i += 4,
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

/// FNV-1a over every word of every kick, a kick's length first.
fn fnv(kicks: &[Vec<u32>]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for k in kicks {
        for w in core::iter::once(k.len() as u32).chain(k.iter().copied()) {
            for b in w.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    h
}

/// The tile-status registers the tiled work added to the stream's vocabulary
/// (9cdeff6d W1 to W5, be0c1527's nine): none is written with the knob off.
fn ts_registers() -> Vec<(String, u32)> {
    let mut v = vec![
        ("TS_COLOR_STATUS_BASE".to_string(), GC_TS_COLOR_STATUS_BASE),
        ("TS_COLOR_SURFACE_BASE".to_string(), GC_TS_COLOR_SURFACE_BASE),
        ("TS_COLOR_CLEAR_VALUE".to_string(), GC_TS_COLOR_CLEAR_VALUE),
        ("TS_COLOR_CLEAR_VALUE_EXT".to_string(), GC_TS_COLOR_CLEAR_VALUE_EXT),
        ("TS_COLOR_AUTO_DISABLE_COUNT".to_string(), GC_TS_COLOR_AUTO_DISABLE_COUNT),
        ("TS_DEPTH_STATUS_BASE".to_string(), GC_TS_DEPTH_STATUS_BASE),
        ("TS_DEPTH_SURFACE_BASE".to_string(), GC_TS_DEPTH_SURFACE_BASE),
        ("TS_DEPTH_CLEAR_VALUE".to_string(), GC_TS_DEPTH_CLEAR_VALUE),
        ("TS_DEPTH_AUTO_DISABLE_COUNT".to_string(), GC_TS_DEPTH_AUTO_DISABLE_COUNT),
    ];
    for i in 0..16 {
        v.push((format!("TS_SAMPLER_CONFIG({i})"), ts_sampler_config(i)));
        v.push((format!("TS_SAMPLER_STATUS_BASE({i})"), ts_sampler_status_base(i)));
    }
    v
}

const FORMATS: [(u8, &str); 2] = [(GLES_SURF_RGBA8888, "RGBA8888"), (GLES_SURF_RGB565, "RGB565")];
const CASES: [Case; 4] = [Case::ScissoredClear, Case::WholeClear, Case::Draw, Case::ClearThenDraw];

#[test]
fn no_tile_status_register_is_written() {
    let regs = ts_registers();
    for (fmt, name) in FORMATS {
        for case in CASES {
            let kicks = streams(fmt, case);
            assert!(!kicks.is_empty(), "{name} {case:?}: no kick");
            for k in &kicks {
                for (reg, val) in writes(k) {
                    if let Some((n, _)) = regs.iter().find(|(_, r)| *r == reg) {
                        panic!("{name} {case:?}: {n} (0x{reg:x}) written with 0x{val:x}");
                    }
                    if reg == GC_PE_DEPTH_CONFIG {
                        assert_eq!(val & GC_PE_DEPTH_EARLY_Z, 0, "{name} {case:?}: PE early depth on");
                    }
                    if reg == GC_RA_EARLY_DEPTH {
                        assert_eq!(val & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0, "{name} {case:?}: RA early depth on");
                    }
                }
            }
        }
    }
}

/// The streams, whole, as 9cdeff6d^ made them. A moved hash is a change of
/// the default path's stream; read the diff of the words before updating it.
#[test]
fn the_streams_are_those_before_the_tiled_work() {
    let mut got = Vec::new();
    for (fmt, name) in FORMATS {
        for case in CASES {
            got.push(format!("{name} {case:?}: 0x{:016x}", fnv(&streams(fmt, case))));
        }
    }
    println!("{}", got.join("\n"));
    let want = [
        "RGBA8888 ScissoredClear: 0x26390607cfcf8716",
        "RGBA8888 WholeClear: 0x73064adbd796764b",
        "RGBA8888 Draw: 0x57ce03a5e9fa535c",
        "RGBA8888 ClearThenDraw: 0xc1d352c4f49ed770",
        "RGB565 ScissoredClear: 0x37386451feff50aa",
        "RGB565 WholeClear: 0x27ece22d5c27602a",
        "RGB565 Draw: 0x6ff67652c67bf060",
        "RGB565 ClearThenDraw: 0x8c72028078416380",
    ];
    assert_eq!(got, want);
}
