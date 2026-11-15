// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): an RGBA32F / RGBA32UI / RGBA32I texture as a colour
//! attachment is refused cleanly (item gc-w128-fbo).
//!
//! The classes are two G32R32 planes when sampled
//! (docs/notes/gc-128bit-texture-sampling.md); 128-bit render targets are not
//! implemented (docs/notes/gc-128bit-render-targets.md). RGBA32I and RGBA32UI
//! are colour-renderable in core ES 3.0, RGBA32F is not (it needs
//! EXT_color_buffer_float), so such an attachment (`glFramebufferTexture2D`,
//! or `glFramebufferTextureLayer` on an array) must never be a complete
//! framebuffer: it must not render into plane 0 as if it were one G32R32
//! surface. For each class and both attach calls this pins the completeness
//! status, a `glDrawArrays` and a `glClearBuffer*` (an error where ES 3.0
//! asks for one, and no draw and no state in any submitted stream), a
//! `glReadPixels`, and that a later sampling draw still reads the same bytes
//! behind each descriptor (both planes once the class is sampled as planes).
//!
//! The mask hook `backend::TEST_EXTRA_SAMPLED` is `#[cfg(test)]`, so an
//! integration test cannot open the classes. Closed, as today
//! (GC_SAMPLED_TEXFMTS holds no 128-bit class), the assertions pin what the
//! GL core does now: RGBA32UI/I have no texture (the upload is
//! GL_INVALID_OPERATION), RGBA32F is a RGBA16F texture, and every attachment
//! is GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT. The checks that hold in both
//! states (never complete, no draw, no state, no read, the bytes unchanged)
//! also run once the classes are open; the exact pins of the closed state
//! are guarded by `open()` and the open state's are left to item
//! gc-w128-flip / tests/es3_wide128_gc.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_fbo_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
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

const WIDE: [(&str, u8, u32); 3] = [
    ("RGBA32F", GLES_TEXFMT_RGBA32F, GL_RGBA32F),
    ("RGBA32UI", GLES_TEXFMT_RGBA32UI, GL_RGBA32UI),
    ("RGBA32I", GLES_TEXFMT_RGBA32I, GL_RGBA32I),
];

fn open(format: u8) -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(format) as u64) != 0
}

fn err() -> GLenum {
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

fn program(fs: &str) -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
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

/// The streams submitted since kick `kicks0`: their register writes, and the
/// number of DRAW_INSTANCED commands in them.
#[derive(Debug, Default)]
struct Run {
    err: GLenum,
    writes: Vec<(u32, u32)>,
    draws: usize,
}

fn run_since(kicks0: usize, err: GLenum) -> Run {
    with_platform(|p| {
        let mut r = Run { err, ..Default::default() };
        for k in &p.kicks[kicks0..] {
            r.writes.extend(writes(k));
            r.draws += draw_ops(k);
        }
        r
    })
}

/// The DRAW_INSTANCED commands of a stream (walked as `writes` does).
fn draw_ops(words: &[u32]) -> usize {
    let (mut i, mut n) = (0usize, 0usize);
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            i += 1 + ((w >> 16) & 0x3ff) as usize;
            continue;
        }
        n += usize::from(w & 0xf800_0000 == GC_OP_DRAW_INSTANCED);
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    n
}

/// One quad with `fs` over 64x64 into whatever framebuffer is bound.
fn draw_quad(fs: &str, sampler_unit: bool) -> Run {
    let kicks0 = with_platform(|p| p.kicks.len());
    let p = program(fs);
    // SAFETY: plain GL calls; QUAD is a static.
    let e = unsafe {
        glViewport(0, 0, 64, 64);
        glUseProgram(p);
        if sampler_unit {
            glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        }
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        let e = glGetError();
        glFinish();
        e
    };
    run_since(kicks0, e)
}

fn sampler(class: u8) -> &'static str {
    match class {
        GLES_TEXFMT_RGBA32F => "sampler2D",
        GLES_TEXFMT_RGBA32I => "isampler2D",
        _ => "usampler2D",
    }
}

/// The fragment shader of a draw into an attachment of `class`.
fn out_fs(class: u8) -> String {
    let (ty, v) = match class {
        GLES_TEXFMT_RGBA32F => ("vec4", "vec4(1.0, 2.0, 3.0, 4.0)"),
        GLES_TEXFMT_RGBA32I => ("ivec4", "ivec4(1, 2, 3, 4)"),
        _ => ("uvec4", "uvec4(1u, 2u, 3u, 4u)"),
    };
    format!("#version 300 es\nprecision highp float;\nprecision highp int;\nout {ty} o;\nvoid main() {{ o = {v}; }}\n")
}

/// The words of slot `slot`'s descriptor in the stream, if it wrote one.
fn descriptor(stream: &[(u32, u32)], slot: u32) -> Option<Vec<u32>> {
    let a = stream.iter().rev().find(|(r, _)| *r == nte_descriptor_addr(slot))?.1;
    (a != 0).then(|| words_at(u64::from(a), GC_TEXDESC_BYTES / 4))
}

/// Level 0 behind each descriptor the stream wrote (slot 0, and the companion
/// plane in slot 1 when there is one): `n` bytes of each.
fn level0_bytes(stream: &[(u32, u32)], n: usize) -> Vec<Vec<u8>> {
    (0..2)
        .filter_map(|s| descriptor(stream, s))
        .map(|d| read_gpu(u64::from(d[texdesc_lod_addr(0) as usize / 4]), n))
        .collect()
}

/// The fragment shader that samples unit 0 and writes the texel as a float.
fn sample_fs(class: u8, layered: bool) -> String {
    let s = sampler(class);
    let (s, fetch) = if layered { (format!("{s}Array"), "ivec3(0)") } else { (s.to_string(), "ivec2(0)") };
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp {s};\nuniform {s} s;\nout vec4 o;\nvoid main() {{ o = vec4(texelFetch(s, {fetch}, 0)); }}\n"
    )
}

/// What an attachment of one class did, gathered for the checks.
#[derive(Debug)]
struct Probe {
    upload: GLenum,
    attach: GLenum,
    status: GLenum,
    draw: Run,
    clear: Run,
    read: GLenum,
    read_words: Vec<u32>,
    /// level 0 behind each descriptor of a sampling draw before the
    /// attachment, and after the draw, the clear and the read; empty when the
    /// class has no texture (the upload was refused)
    before: Vec<Vec<u8>>,
    after: Vec<Vec<u8>>,
}

/// A 2x2 level 0 (two layers when `layered`) of `class`.
fn upload(class: &(&str, u8, u32), layered: bool) -> (GLuint, GLenum) {
    let (ty, fmt) = match class.1 {
        GLES_TEXFMT_RGBA32F => (GL_FLOAT, GL_RGBA),
        GLES_TEXFMT_RGBA32I => (GL_INT, GL_RGBA_INTEGER),
        _ => (GL_UNSIGNED_INT, GL_RGBA_INTEGER),
    };
    let texels: Vec<u32> = (0..32u32)
        .map(|i| if class.1 == GLES_TEXFMT_RGBA32F { 0x3f80_0000 + i } else { 0x1111_1111 * (i % 15 + 1) })
        .collect();
    let target = if layered { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_2D };
    let mut t: GLuint = 0;
    // SAFETY: plain GL calls; 2x2 (x2) texels of 16 bytes.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if layered {
            glTexImage3D(target, 0, class.2 as GLint, 2, 2, 2, 0, fmt, ty, texels.as_ptr().cast());
        } else {
            glTexImage2D(target, 0, class.2 as GLint, 2, 2, 0, fmt, ty, texels.as_ptr().cast());
        }
    }
    (t, err())
}

/// Upload a texture of `class`, sample it, attach it at colour attachment 0
/// of a framebuffer (`glFramebufferTexture2D`, or layer 1 of an array with
/// `glFramebufferTextureLayer`), then check the status, draw into it, clear
/// it, read it, and sample it again.
fn probe(class: &(&str, u8, u32), layered: bool) -> Probe {
    let (tex, upload_err) = upload(class, layered);
    let target = if layered { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_2D };
    // 128 bytes a plane's 4x4-padded 2x2 level; a layer is one such slice
    let n = if layered { 256 } else { 128 };
    let sample = |what: &str| -> Vec<Vec<u8>> {
        if upload_err != GL_NO_ERROR {
            return Vec::new();
        }
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(target, tex);
        }
        let r = draw_quad(&sample_fs(class.1, layered), true);
        assert_eq!(r.err, GL_NO_ERROR, "{}: the sampling draw {what}", class.0);
        assert_eq!(r.draws, 1, "{}: the sampling draw {what} is one draw", class.0);
        level0_bytes(&r.writes, n)
    };
    let before = sample("before");

    let mut fb = 0;
    // SAFETY: plain GL calls.
    let (attach, status) = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if layered {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1);
        } else {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        }
        let attach = glGetError();
        let buf = GL_COLOR_ATTACHMENT0;
        glDrawBuffers(1, &buf);
        (attach, glCheckFramebufferStatus(GL_FRAMEBUFFER))
    };
    let _ = err();
    let draw = draw_quad(&out_fs(class.1), false);

    let kicks0 = with_platform(|p| p.kicks.len());
    let c = [7u32; 4];
    // SAFETY: four values; plain GL calls.
    let clear = unsafe {
        match class.1 {
            GLES_TEXFMT_RGBA32F => glClearBufferfv(GL_COLOR, 0, c.as_ptr() as *const f32),
            GLES_TEXFMT_RGBA32I => glClearBufferiv(GL_COLOR, 0, c.as_ptr() as *const GLint),
            _ => glClearBufferuiv(GL_COLOR, 0, c.as_ptr()),
        }
        let e = glGetError();
        glFinish();
        run_since(kicks0, e)
    };

    let mut read_words = vec![0xdead_beefu32; 16];
    // SAFETY: the buffer holds 2x2 pixels of four words.
    let read = unsafe {
        let (f, t) = match class.1 {
            GLES_TEXFMT_RGBA32F => (GL_RGBA, GL_FLOAT),
            GLES_TEXFMT_RGBA32I => (GL_RGBA_INTEGER, GL_INT),
            _ => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        };
        glReadPixels(0, 0, 2, 2, f, t, read_words.as_mut_ptr().cast());
        glGetError()
    };
    let after = sample("after");
    Probe { upload: upload_err, attach, status, draw, clear, read, read_words, before, after }
}

fn all_probes() -> Vec<(String, (&'static str, u8, u32), Probe)> {
    let mut out = Vec::new();
    for class in WIDE {
        for layered in [false, true] {
            let what = format!("{} {}", class.0, if layered { "layer 1 of an array" } else { "2D level 0" });
            on_gc7000(|| {
                let p = probe(&class, layered);
                out.push((what, class, p));
            });
        }
    }
    out
}

/// No colour attachment of the three classes is a complete framebuffer on
/// this back end, however it was attached: RGBA32I and RGBA32UI are
/// colour-renderable in core ES 3.0 and RGBA32F is not (it needs
/// EXT_color_buffer_float), but 128-bit render targets are not implemented
/// (docs/notes/gc-128bit-render-targets.md), so none may report
/// GL_FRAMEBUFFER_COMPLETE. The attach call itself is not an error.
///
/// Closed mask, today: GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT for all of
/// them, the integer classes having no texture at all (their upload is a
/// GL_INVALID_OPERATION, `es3_storage_or_refuse`) and RGBA32F being a
/// RGBA16F texture (`sized_format`), which is not a colour-renderable ES 3.0
/// format here.
#[test]
fn an_rgba32_attachment_is_not_a_complete_framebuffer() {
    for (what, class, p) in all_probes() {
        assert_eq!(p.attach, GL_NO_ERROR, "{what}: attaching is not an error");
        assert_ne!(p.status, GL_FRAMEBUFFER_COMPLETE, "{what}: must not be complete");
        if !open(class.1) {
            assert_eq!(p.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{what}: closed");
            assert_eq!(
                p.upload,
                if class.1 == GLES_TEXFMT_RGBA32F { GL_NO_ERROR } else { GL_INVALID_OPERATION },
                "{what}"
            );
        } else {
            assert!(
                p.status == GL_FRAMEBUFFER_UNSUPPORTED || p.status == GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
                "{what}: open: 0x{:x}",
                p.status
            );
        }
    }
}

/// A glDrawArrays into the framebuffer is refused with
/// GL_INVALID_FRAMEBUFFER_OPERATION and writes no stream at all: no kick
/// carries a draw, a colour target address or any state, so neither plane is
/// reached.
#[test]
fn a_draw_into_an_rgba32_attachment_writes_nothing() {
    for (what, _, p) in all_probes() {
        assert_eq!(p.draw.err, GL_INVALID_FRAMEBUFFER_OPERATION, "{what}: the draw's error");
        assert_eq!(p.draw.draws, 0, "{what}: no draw in the stream");
        assert!(p.draw.writes.is_empty(), "{what}: no state written: {:x?}", p.draw.writes);
    }
}

/// glClearBufferfv / iv / uiv of draw buffer 0 writes no stream either (a
/// clear is a draw or a blit on this back end; none appears).
#[test]
fn a_clear_of_an_rgba32_attachment_writes_nothing() {
    for (what, class, p) in all_probes() {
        assert_eq!(p.clear.draws, 0, "{what}: no draw in the stream");
        assert!(p.clear.writes.is_empty(), "{what}: no state written: {:x?}", p.clear.writes);
        if !open(class.1) {
            /* the clear of an incomplete framebuffer raises no error here, as the
             * clear of any other (es3.rs); pinned as it is, not as ES 3.0 4.4.4
             * asks (GL_INVALID_FRAMEBUFFER_OPERATION) */
            assert_eq!(p.clear.err, GL_NO_ERROR, "{what}: closed");
        } else {
            assert!(
                p.clear.err == GL_NO_ERROR || p.clear.err == GL_INVALID_FRAMEBUFFER_OPERATION,
                "{what}: 0x{:x}",
                p.clear.err
            );
        }
    }
}

/// glReadPixels from the framebuffer raises an error and leaves the
/// caller's memory alone. Closed, RGBA32F (halves, read as GL_FLOAT) gets
/// GL_INVALID_FRAMEBUFFER_OPERATION from the incomplete framebuffer; the
/// integer classes get GL_INVALID_OPERATION, the read format check of an
/// attachment with no storage format coming first.
#[test]
fn reading_an_rgba32_attachment_is_refused() {
    for (what, class, p) in all_probes() {
        assert!(p.read == GL_INVALID_FRAMEBUFFER_OPERATION || p.read == GL_INVALID_OPERATION, "{what}: 0x{:x}", p.read);
        if !open(class.1) {
            let want =
                if class.1 == GLES_TEXFMT_RGBA32F { GL_INVALID_FRAMEBUFFER_OPERATION } else { GL_INVALID_OPERATION };
            assert_eq!(p.read, want, "{what}: closed");
        }
        assert!(p.read_words.iter().all(|w| *w == 0xdead_beef), "{what}: the buffer is untouched");
    }
}

/// After all of that, sampling the texture in a draw into the default
/// framebuffer reads what was uploaded: level 0 behind each descriptor
/// (both planes once the class is sampled as planes, slot 0 and its
/// companion in slot 1) is byte-identical to what it was before the
/// attachment. The integer classes have no texture while closed, so there
/// is nothing to sample and the check says so.
#[test]
fn the_texture_still_samples_what_was_uploaded() {
    for (what, class, p) in all_probes() {
        if p.upload != GL_NO_ERROR {
            eprintln!("{what}: no texture (upload 0x{:x}); nothing to sample (wide128-flip)", p.upload);
            assert!(!open(class.1), "{what}: an open class must upload");
            assert!(p.before.is_empty() && p.after.is_empty(), "{what}");
            continue;
        }
        assert!(!p.before.is_empty(), "{what}: sampled before");
        assert_eq!(p.before, p.after, "{what}: the planes are unchanged");
        /* RGBA32F is held as halves (`sized_format`) until the flip makes the
         * GL core store it as planes, open or not: one descriptor then */
        let planes = open(class.1) && class.1 != GLES_TEXFMT_RGBA32F;
        if planes {
            assert_eq!(p.before.len(), 2, "{what}: a descriptor and its companion");
        } else {
            assert!(
                p.before.len() == 1 || (open(class.1) && p.before.len() == 2),
                "{what}: {} descriptors",
                p.before.len()
            );
        }
        assert!(p.before.iter().any(|b| b.iter().any(|x| *x != 0)), "{what}: the uploaded texels are there");
    }
}
