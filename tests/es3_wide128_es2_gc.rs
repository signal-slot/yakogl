// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): an ES 2.0 surface keeps GL_RGBA32F out of the
//! two-plane class (docs/notes/gc-128bit-texture-sampling.md).
//!
//! ES 2.0 is the cargo feature `report-es2` (`crate::REPORT_ES2`), not a
//! second kind of context: the harness opens the same context either way, so
//! the two builds are the two surfaces. The flip of item wide128-flip puts
//! RGBA32F/UI/I into GC_SAMPLED_TEXFMTS so ES 3.0 contexts store and sample
//! them as two G32R32 planes. What an ES 2.0 build does with the same names
//! must not move:
//!
//! - `report-es2`: GL_OES_texture_float is not claimed, so `glTexImage2D`
//!   refuses GL_RGBA32F and the other RGBA32 names, and an unsized
//!   GL_RGBA/GL_FLOAT upload, all with GL_INVALID_ENUM (their types are ES 3.0
//!   ones, not this surface's). `glTexStorage2D`
//!   (an ES 3.0 entry point; `sized_format` is shared) takes GL_RGBA32F and
//!   holds it as RGBA16F halves: the draw's descriptor is the A16B16G16R16F
//!   word, one descriptor, no companion plane in slot 1, and the program is
//!   the ordinary one-sampler variant.
//! - the ES 3.0 build: an unsized GL_RGBA/GL_FLOAT upload is the RGBA8 store
//!   (clamped to unorm8), never a 128-bit class; that does not move either.
//!
//! The ES 2.0 shaders are GLSL ES 1.00 (a report-es2 build refuses 3.00).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es3_wide128_es2_gc
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_es2_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{OsalGpuMem, OsalOps, osal_set_ops};
use yakogl::vivante::kick::{GC_KICK_MAGIC, GcKick};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL (as tests/gc_tiled_copy_reuse.rs) ---------------- */

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
        p.allocs.insert(gpu, Alloc { mem, size });
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

/// `n` bytes of the stand-in's memory at GPU address `gpu`.
fn read_gpu(gpu: u64, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back().unwrap_or_else(|| panic!("no allocation at 0x{gpu:x}"));
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} runs off the allocation at 0x{base:x} ({} bytes)", a.size);
        // SAFETY: inside the allocation (checked just above).
        unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
}

fn words_at(gpu: u64, n: usize) -> Vec<u32> {
    let b = read_gpu(gpu, n * 4);
    let (words, _) = b.as_chunks::<4>();
    words.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

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

/* ---- GL plumbing -------------------------------------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

#[cfg(feature = "report-es2")]
fn error() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
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

/// A program sampling unit 0 at its first texel, in the dialect of the build:
/// GLSL ES 1.00 under `report-es2`, which refuses 3.00, else 3.00.
fn program() -> GLuint {
    let (vs, fs) = if cfg!(feature = "report-es2") {
        (
            "attribute vec4 p;\nvoid main() { gl_Position = p; }\n",
            "precision highp float;\nuniform sampler2D s;\nvoid main() { gl_FragColor = texture2D(s, vec2(0.25)); }\n",
        )
    } else {
        (
            "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n",
            "#version 300 es\nprecision highp float;\nuniform sampler2D s;\nout vec4 o;\nvoid main() { o = texture(s, vec2(0.25)); }\n",
        )
    };
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The words of slot `slot`'s descriptor in the last kick, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// One quad sampling unit 0 with program `p`: the last kick's writes.
fn draw(p: GLuint) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    no_error("glFinish");
    let stream = with_platform(|p| {
        assert!(p.kicks.len() > kicks0, "the quad produced no kick");
        p.kicks.last().cloned().unwrap()
    });
    writes(&stream)
}

fn bind_new() -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        t
    }
}

/// The unit-0 draw's format word and whether a companion plane follows it.
fn sampled_word(stream: &[(u32, u32)]) -> (u32, u32, bool) {
    let d = descriptor(stream, 0).expect("a descriptor in slot 0");
    (d[GC_TEXDESC_CONFIG0 as usize / 4], d[GC_TEXDESC_CONFIG1 as usize / 4] & 0x3f, descriptor(stream, 1).is_some())
}

#[cfg(feature = "report-es2")]
const FORMAT_FIELD: u32 = 0x1f << 13;

/// The names of the 128-bit class: refused by `glTexImage2D` on an ES 2.0
/// surface, with the unsized float pair beside them.
#[test]
#[cfg(feature = "report-es2")]
fn the_es2_surface_refuses_the_rgba32_names_and_unsized_float() {
    on_gc7000(|| {
        let _t = bind_new();
        let texel = [0u8; 16];
        for (internal, format, ty) in [
            (GL_RGBA32F, GL_RGBA, GL_FLOAT),
            (GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT),
            (GL_RGBA32I, GL_RGBA_INTEGER, GL_INT),
        ] {
            // SAFETY: one 16-byte texel.
            unsafe { glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 1, 1, 0, format, ty, texel.as_ptr().cast()) };
            assert_eq!(error(), GL_INVALID_ENUM, "internalformat 0x{internal:x}");
        }
        // SAFETY: as above.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 1, 1, 0, GL_RGBA, GL_FLOAT, texel.as_ptr().cast()) };
        assert_eq!(error(), GL_INVALID_ENUM, "unsized GL_RGBA/GL_FLOAT");
        // SAFETY: as above.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 1, 1, 0, GL_RGBA, GL_FLOAT, core::ptr::null()) };
        assert_eq!(error(), GL_INVALID_ENUM, "unsized GL_RGBA/GL_FLOAT, no data");
    });
}

/// `glTexStorage2D(GL_RGBA32F)` on an ES 2.0 surface holds halves: the
/// draw's one descriptor is the A16B16G16R16F word, and the 128-bit class's
/// companion plane is not there.
#[test]
#[cfg(feature = "report-es2")]
fn rgba32f_storage_on_the_es2_surface_is_halves_with_no_companion() {
    on_gc7000(|| {
        let _t = bind_new();
        // SAFETY: plain GL call.
        unsafe { glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA32F, 8, 8) };
        let e = error();
        if e != GL_NO_ERROR {
            /* a surface that refuses the entry point stores nothing: pinned as it is */
            assert_eq!(e, GL_INVALID_OPERATION, "glTexStorage2D(GL_RGBA32F)");
            eprintln!("glTexStorage2D(GL_RGBA32F) is refused here with 0x{e:x}; the stored class is not drawn");
            return;
        }
        let s = draw(program());
        let (c0, ext, companion) = sampled_word(&s);
        assert_eq!(c0 & FORMAT_FIELD, 0, "the extended format field names the format");
        assert_eq!(ext, GC_TEX_FORMAT_EXT_A16B16G16R16F, "RGBA32F is held as RGBA16F halves");
        assert!(!companion, "no companion plane descriptor in slot 1");
    });
}

/// The same on the ES 3.0 build, which is not this item's subject but shares
/// the pin that does not move: an unsized RGBA/FLOAT upload is RGBA8, with
/// no 128-bit word and no companion.
#[test]
#[cfg(not(feature = "report-es2"))]
fn an_unsized_float_upload_stays_rgba8_with_no_companion() {
    on_gc7000(|| {
        let _t = bind_new();
        let texels: Vec<u8> = [0.0f32, 0.5, 1.0, 2.0].iter().cycle().take(16).flat_map(|f| f.to_le_bytes()).collect();
        // SAFETY: a 2x2 image of four floats a texel (64 bytes).
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 2, 2, 0, GL_RGBA, GL_FLOAT, texels.as_ptr().cast()) };
        no_error("unsized RGBA/FLOAT");
        let s = draw(program());
        let (_, ext, companion) = sampled_word(&s);
        assert_ne!(ext, GC_TEX_FORMAT_EXT_G32R32F, "not the 128-bit float word");
        assert_ne!(ext, GC_TEX_FORMAT_EXT_G32R32I, "not the 128-bit integer word");
        assert_ne!(ext, GC_TEX_FORMAT_EXT_A16B16G16R16F, "not halves either: the RGBA8 store");
        assert!(!companion, "no companion plane descriptor in slot 1");
    });
}
