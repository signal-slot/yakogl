// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): what the ES 3.0 queries say about the 128-bit texture
//! classes, RGBA32F / RGBA32UI / RGBA32I and RGB32F / RGB32I / RGB32UI /
//! RG32F, through the GL API on the stand-in OSAL
//! (as tests/es3_wide128_planes_gc.rs).
//!
//! The answers are the same whether or not GC_SAMPLED_TEXFMTS holds the
//! 128-bit classes (the coming RGBA32 flip, docs/notes/gc-128bit-texture-sampling.md):
//! they are plain asserts with no `open()` branch, and none of them reads the
//! sampled mask. Pinned here:
//!
//! - glGetInternalformativ (GL_NUM_SAMPLE_COUNTS, GL_SAMPLES): the renderbuffer
//!   query, the only target ES 3.0 6.1.15 allows;
//! - glGetTexParameteriv GL_TEXTURE_IMMUTABLE_FORMAT / _LEVELS before and after
//!   glTexStorage2D, and that glTexImage2D leaves them at 0;
//! - glGetError after glTexImage2D, glTexStorage2D and glTexSubImage2D with
//!   each valid format/type pair of docs/es3/sized-formats.md, and with pairs
//!   the format does not take.
//!
//! The format-by-format accept matrix itself is tests/es3_sized_formats.rs.
//!
//! Never under `report-es2`, which keeps its ES 2.0 answers: there
//! glTexStorage2D is no command (GL_INVALID_OPERATION) and none of these
//! names is a format, so this is an ES 3.0 build's test (as
//! tests/es3_wide128_narrow_gc.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_query_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL (as tests/gc_tiled_copy_reuse.rs) ---------------- */

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
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
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
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

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
        p.kicks.push(words.to_vec());
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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- the classes and their valid pairs ----------------------------------- */

/// (name, internalformat, format, type, texel bytes on the host side).
const FORMATS: [(&str, u32, u32, u32, usize); 7] = [
    ("RGBA32F", GL_RGBA32F, GL_RGBA, GL_FLOAT, 16),
    ("RGBA32UI", GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, 16),
    ("RGBA32I", GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, 16),
    ("RGB32F", GL_RGB32F, GL_RGB, GL_FLOAT, 12),
    ("RGB32I", GL_RGB32I, GL_RGB_INTEGER, GL_INT, 12),
    ("RGB32UI", GL_RGB32UI, GL_RGB_INTEGER, GL_UNSIGNED_INT, 12),
    ("RG32F", GL_RG32F, GL_RG, GL_FLOAT, 8),
];

/// The float formats: stored today as RGBA16F, as RGBA32F after the flip, and
/// accepted either way. The integer ones are refused today (no sampled class
/// holds them) and accepted after it, so only `integer_classes_either_way`
/// looks at them.
fn is_float(f: u32) -> bool {
    matches!(f, GL_RGBA32F | GL_RGB32F | GL_RG32F)
}

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn clean(what: &str) {
    let e = err();
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn tex() -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    clean("bind");
    t
}

fn param(pname: GLenum) -> GLint {
    let mut v: GLint = -77;
    // SAFETY: plain GL call, one value.
    unsafe { glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut v) };
    clean("glGetTexParameteriv");
    v
}

/// Zeroed host data for `w * h` texels of a pair, with room to spare.
fn data(w: usize, h: usize, texel: usize) -> Vec<u8> {
    vec![0u8; w * h * texel + 16]
}

/// glGetInternalformativ on a renderbuffer: the error, NUM_SAMPLE_COUNTS and
/// the GL_SAMPLES list (an 8-entry buffer filled with -1; unwritten entries stay).
fn sample_query(internalformat: u32) -> (GLenum, GLint, Vec<GLint>) {
    clean("before the query");
    let mut n: GLint = -1;
    let mut s = [-1 as GLint; 8];
    // SAFETY: plain GL calls; the buffers hold the sizes passed.
    unsafe {
        glGetInternalformativ(GL_RENDERBUFFER, internalformat, GL_NUM_SAMPLE_COUNTS, 1, &mut n);
        let e1 = glGetError();
        glGetInternalformativ(GL_RENDERBUFFER, internalformat, GL_SAMPLES, 8, s.as_mut_ptr());
        let e2 = glGetError();
        assert_eq!(e1, e2, "{internalformat:#x}: the two pnames agree on the error");
        let k = s.iter().take_while(|&&v| v != -1).count();
        (e1, n, s[..k].to_vec())
    }
}

/* ---- tests -------------------------------------------------------------- */

/// The renderbuffer query. RGBA32UI/I are renderable formats of table 3.13 the
/// query accepts, with no multisample count (an integer format has none);
/// RGBA32F, the RGB32* and RG32F are not colour-renderable in ES 3.0 and the
/// query refuses them with INVALID_ENUM, writing nothing.
#[test]
fn internalformat_samples() {
    on_gc7000(|| {
        for (name, f, ..) in FORMATS {
            let (e, n, list) = sample_query(f);
            match f {
                GL_RGBA32UI | GL_RGBA32I => {
                    assert_eq!(e, GL_NO_ERROR, "{name}: renderable, the query is accepted");
                    assert_eq!(n, 0, "{name}: no sample counts");
                    assert!(list.is_empty(), "{name}: GL_SAMPLES writes nothing: {list:?}");
                }
                _ => {
                    assert_eq!(e, GL_INVALID_ENUM, "{name}: not a renderable format");
                    assert_eq!(n, -1, "{name}: NUM_SAMPLE_COUNTS is left alone");
                    assert!(list.is_empty(), "{name}: GL_SAMPLES writes nothing");
                }
            }
        }
        /* a texture target is no target of the query, whatever the format */
        for (name, f, ..) in FORMATS {
            let mut v: GLint = -1;
            // SAFETY: plain GL call, one value.
            unsafe { glGetInternalformativ(GL_TEXTURE_2D, f, GL_NUM_SAMPLE_COUNTS, 1, &mut v) };
            assert_eq!(err(), GL_INVALID_ENUM, "{name}: GL_TEXTURE_2D");
            assert_eq!(v, -1, "{name}");
        }
    });
}

/// TEXTURE_IMMUTABLE_FORMAT / _LEVELS: 0 on a fresh texture, the flag and the
/// level count after glTexStorage2D, and the storage stays final.
#[test]
fn immutable_format_and_levels() {
    on_gc7000(|| {
        for (name, f, ..) in FORMATS.into_iter().filter(|r| is_float(r.1)) {
            let _t = tex();
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_FORMAT), 0, "{name}: fresh");
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_LEVELS), 0, "{name}: fresh");
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 4, f, 8, 8) };
            clean(&format!("{name}: glTexStorage2D"));
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_FORMAT), 1, "{name}: after storage");
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_LEVELS), 4, "{name}: after storage");
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 4, f, 8, 8) };
            assert_eq!(err(), GL_INVALID_OPERATION, "{name}: storage twice");
            let d = data(8, 8, 16);
            // SAFETY: `d` holds an 8x8 image of any pair.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, f as GLint, 8, 8, 0, GL_RGBA, GL_FLOAT, d.as_ptr() as *const c_void)
            };
            assert_eq!(err(), GL_INVALID_OPERATION, "{name}: glTexImage2D on immutable storage");
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_LEVELS), 4, "{name}: unchanged");

            let _t = tex();
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, f, 5, 3) };
            clean(&format!("{name}: one level"));
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_LEVELS), 1, "{name}: one level");
        }
    });
}

/// glTexImage2D / glTexSubImage2D with each format's valid pair: no error and
/// the texture stays mutable; pairs the format does not take are
/// INVALID_OPERATION.
#[test]
fn upload_with_each_valid_pair() {
    on_gc7000(|| {
        for (name, f, format, ty, texel) in FORMATS.into_iter().filter(|r| is_float(r.1)) {
            let _t = tex();
            let d = data(6, 5, texel);
            let dp = d.as_ptr() as *const c_void;
            // SAFETY: `d` holds 6x5 texels of the pair.
            unsafe { glTexImage2D(GL_TEXTURE_2D, 0, f as GLint, 6, 5, 0, format, ty, dp) };
            clean(&format!("{name}: glTexImage2D"));
            // SAFETY: as above, a 3x2 sub-rectangle.
            unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 3, 2, format, ty, dp) };
            clean(&format!("{name}: glTexSubImage2D"));
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_FORMAT), 0, "{name}: glTexImage2D keeps it mutable");
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_LEVELS), 0, "{name}");

            /* storage then a sub-image of the same pair */
            let _t = tex();
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, f, 6, 5) };
            clean(&format!("{name}: glTexStorage2D"));
            // SAFETY: `d` holds the 6x5 texels.
            unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 6, 5, format, ty, dp) };
            clean(&format!("{name}: sub-image into storage"));
            // SAFETY: refused before it reads.
            unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 4, 0, 3, 1, format, ty, dp) };
            assert_eq!(err(), GL_INVALID_VALUE, "{name}: sub-image past the level");

            /* the pairs it does not take */
            let _t = tex();
            for (bf, bt) in
                [(GL_RGBA_INTEGER, ty), (GL_RGB_INTEGER, ty), (GL_RED_INTEGER, ty), (GL_DEPTH_COMPONENT, ty)]
            {
                if (bf, bt) == (format, ty) {
                    continue;
                }
                // SAFETY: refused before it reads.
                unsafe { glTexImage2D(GL_TEXTURE_2D, 0, f as GLint, 6, 5, 0, bf, bt, dp) };
                assert_eq!(err(), GL_INVALID_OPERATION, "{name}: format {bf:#x} type {bt:#x}");
            }
        }
    });
}

/// The integer 128-bit classes (RGBA32UI/I, RGB32UI/I): today the GL core
/// refuses them (no sampled class holds them), after the flip it stores them.
/// What does not move: the one error is INVALID_OPERATION, a refusal makes
/// nothing (no immutable flag), a success makes it, and the flag and the
/// level count always agree.
#[test]
fn integer_classes_either_way() {
    on_gc7000(|| {
        for (name, f, ..) in FORMATS.into_iter().filter(|r| !is_float(r.1)) {
            let _t = tex();
            // SAFETY: plain GL call.
            unsafe { glTexStorage2D(GL_TEXTURE_2D, 3, f, 4, 4) };
            let e = err();
            assert!(e == GL_NO_ERROR || e == GL_INVALID_OPERATION, "{name}: storage: 0x{e:x}");
            let made = e == GL_NO_ERROR;
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_FORMAT), made as GLint, "{name}");
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_LEVELS), if made { 3 } else { 0 }, "{name}");
        }
    });
}

/// glTexStorage2D's argument errors for the 128-bit classes are the common
/// ones: no levels, no width, more levels than the chain has.
#[test]
fn storage_argument_errors() {
    on_gc7000(|| {
        for (name, f, ..) in FORMATS {
            let _t = tex();
            // SAFETY: plain GL calls; each is refused.
            unsafe {
                glTexStorage2D(GL_TEXTURE_2D, 0, f, 8, 8);
                assert_eq!(err(), GL_INVALID_VALUE, "{name}: 0 levels");
                glTexStorage2D(GL_TEXTURE_2D, 1, f, 0, 8);
                assert_eq!(err(), GL_INVALID_VALUE, "{name}: width 0");
                glTexStorage2D(GL_TEXTURE_2D, 5, f, 8, 8);
                assert_eq!(err(), GL_INVALID_OPERATION, "{name}: 5 levels of 8x8");
            }
            assert_eq!(param(GL_TEXTURE_IMMUTABLE_FORMAT), 0, "{name}: nothing was made");
        }
    });
}
